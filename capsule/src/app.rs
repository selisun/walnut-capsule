//! 主窗口：无边框置顶、层叠透明 DIB、系统托盘。
//!
//! 架构：
//!   - 1 个 DWM 顶层窗口（WS_POPUP、WS_EX_TOPMOST、WS_EX_LAYERED），
//!     全部绘制由 tiny-skia → DIB → UpdateLayeredWindow 完成；
//!   - 鼠标拖动：WM_LBUTTONDOWN 捕获 → WM_MOUSEMOVE 移动 → WM_LBUTTONUP；
//!     移动距离 < 4px 视为点击，触发展开/收起；
//!   - 展开时 SetActiveWindow 以接收 Esc / WM_KILLFOCUS（点击外部收起）。

use crate::config::{CardVis, Config};
use crate::data::*;
use crate::font::Font;
use crate::ui::capsule_view::{self, CapsuleView};
use crate::ui::View;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::raw::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, RwLock};
use std::time::Instant;

#[allow(dead_code)]
fn dbg_log(s: &str) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open("C:/Users/nmche/AppData/Local/Temp/htc_debug.log")
        .and_then(|mut f| writeln!(f, "{}.{:03} {}", t.as_secs() % 10000, t.subsec_millis(), s));
}
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::Networking::WinSock::{WSAStartup, WSADATA};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// 鼠标离开窗口（部分 feature 组合下该常量不导出，这里直接给出值）。
const WM_MOUSELEAVE: u32 = 0x02A3;

fn pcwstr(s: &str) -> PCWSTR {
    let v: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
    let leaked: &'static [u16] = Box::leak(v.into_boxed_slice());
    PCWSTR(leaked.as_ptr())
}

/// GetProcAddress 用 ANSI 名称。
fn pcstr(s: &str) -> windows::core::PCSTR {
    let v: Vec<u8> = s.bytes().chain(Some(0)).collect();
    let leaked: &'static [u8] = Box::leak(v.into_boxed_slice());
    windows::core::PCSTR(leaked.as_ptr())
}

/// 缓存版宽字符串：用于轮询路径（如每 500ms 查任务栏），避免 pcwstr 反复泄漏。
fn static_pcwstr(s: &'static str) -> PCWSTR {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static MAP: OnceLock<Mutex<HashMap<&'static str, &'static [u16]>>> = OnceLock::new();
    let mut g = MAP
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap();
    let v = g.entry(s).or_insert_with(|| {
        Box::leak(
            s.encode_utf16()
                .chain(Some(0))
                .collect::<Vec<u16>>()
                .into_boxed_slice(),
        )
    });
    PCWSTR(v.as_ptr())
}

fn class_name() -> PCWSTR {
    static WIDE: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let v = WIDE.get_or_init(|| "HeTaoCapsule0".encode_utf16().chain(Some(0)).collect());
    PCWSTR(v.as_ptr())
}

fn window_title() -> PCWSTR {
    static WIDE: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let v = WIDE.get_or_init(|| "核桃胶囊".encode_utf16().chain(Some(0)).collect());
    PCWSTR(v.as_ptr())
}

/// rsrc 写入 PE 资源的 RT_GROUP_ICON 编号（rsrc 工具固定为 1）。
const APP_ICON_ID: u16 = 1;

/// MAKEINTRESOURCEW：整数资源 ID 转名字指针（windows 0.58 未导出该宏）。
fn make_int_resource(id: u16) -> PCWSTR {
    PCWSTR(id as usize as *const u16)
}

/// 应用图标：(大图标 窗口类/Alt-Tab, 小图标 托盘/标题栏)。
/// 从嵌入 PE 的 ico 资源组按尺寸选取，进程期常驻、LR_SHARED 无需销毁。
fn app_icons() -> (HICON, HICON) {
    // HICON 内含裸指针，不是 Send/Sync，用 Once + static mut 做一次性初始化
    static ONCE: std::sync::Once = std::sync::Once::new();
    static mut ICONS: (HICON, HICON) = HICON_NULL_PAIR;
    unsafe {
        ONCE.call_once(|| {
            let inst = GetModuleHandleW(None).unwrap_or_default();
            let res = make_int_resource(APP_ICON_ID);
            let to_icon = |h: HANDLE| HICON(h.0);
            let big = LoadImageW(
                HINSTANCE(inst.0),
                res,
                IMAGE_ICON,
                0,
                0,
                IMAGE_FLAGS(LR_DEFAULTSIZE.0 | LR_SHARED.0),
            )
            .map(to_icon)
            .unwrap_or_default();
            let (sx, sy) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
            let small = LoadImageW(
                HINSTANCE(inst.0),
                res,
                IMAGE_ICON,
                sx,
                sy,
                IMAGE_FLAGS(LR_SHARED.0),
            )
            .map(to_icon)
            .unwrap_or(big);
            ICONS = (big, small);
        });
        ICONS
    }
}

/// HICON 不能用于 const 构造，用该常量占位初始化 static mut。
const HICON_NULL_PAIR: (HICON, HICON) = (HICON(std::ptr::null_mut()), HICON(std::ptr::null_mut()));

/// 任务栏内三环窗口的尺寸（固定到任务栏时的收起态尺寸）。
const CAPSULE_W: u32 = capsule_view::RINGS_W;
/// 初始窗口高度（未固定时药丸胶囊的高度；固定态由三环窗口尺寸决定）。
const CAPSULE_H: u32 = 52;

/// 主入口。阻塞直到进程退出。
pub fn run(config: Config) {
    // UIA 客户端需要 COM；UI 线程消息循环本就按 STA 运行，这里初始化一次。
    // 已被别的模式初始化时返回 RPC_E_CHANGED_MODE，忽略即可。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    // 定时器精度提到 1ms：展开动画与帧节拍更稳，并缩短 UI 线程被高优先级线程
    // 抢占后的最坏等待（默认 15.6ms 粒度下一次抢占就可能让单帧破 6ms 红线）。
    // 本进程空闲时完全阻塞在消息循环，不额外耗 CPU。
    unsafe {
        let _ = timeBeginPeriod(1);
        // UI 线程略提优先级，压低 DWM/其他进程造成的帧内调度抖动；
        // 面板构建线程保持 BELOW_NORMAL，确保光栅化不抢占 UI 帧。
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
    }

    // winsock 首次初始化必须在建窗口之前于本线程同步完成：
    // WSAStartup 会加载 mswsock 并枚举协议目录，若与窗口首显（GDI/IME/TSF/DWM
    // 等系统组件首次初始化）并发，实测会触发系统侧约 68MB 的异常内存提交；
    // 串行预热后该问题消失。
    unsafe {
        let mut wsa: WSADATA = std::mem::zeroed();
        let _ = WSAStartup(0x0202, &mut wsa);
    }

    let instance = unsafe { GetModuleHandleW(None).unwrap_or_default() };

    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: instance.into(),
        lpszClassName: class_name(),
        hIcon: app_icons().0,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        hbrBackground: HBRUSH(null_mut()),
        ..Default::default()
    };
    unsafe {
        let _ = RegisterClassW(&wc);
    }

    // 任务栏内三环子窗口使用独立的窗口类（不同 wndproc）。
    let wc_rings = WNDCLASSW {
        lpfnWndProc: Some(rings_wndproc),
        hInstance: instance.into(),
        lpszClassName: rings_class_name(),
        hCursor: unsafe { LoadCursorW(None, IDC_HAND).unwrap_or_default() },
        hbrBackground: HBRUSH(null_mut()),
        ..Default::default()
    };
    unsafe {
        let _ = RegisterClassW(&wc_rings);
    }

    let shared = Arc::new(RwLock::new(SharedState::default()));
    {
        // 启动即把卡片开关同步给渲染侧（UI 线程与面板构建线程共用这一份）
        let mut sh = match shared.write() {
            Ok(g) => g,
            Err(e) => {
                eprintln!("[App] RwLock poisoned during init: {:?}", e);
                return;
            }
        };
        sh.nas_host = config.nas.host.clone();
        sh.dark = config.capsule.theme_dark();
        sh.cards = config.capsule.cards;
        sh.big = config.capsule.expand_big();
    }
    let (cmd_tx, cmd_rx) = channel::<SourceCmd>();

    let hwnd = create_window(instance, (initial_w(&config), CAPSULE_H));
    if hwnd.is_invalid() {
        return;
    }
    set_initial_pos(hwnd, &config);

    tray_init(hwnd, app_icons().1);

    let view = Box::new(CapsuleView::new(Arc::new(
        std::sync::atomic::AtomicIsize::new(hwnd.0 as isize),
    )));
    let font = Font::load();
    // 预热界面用字字形缓存（启动期一次性冷光栅化，避免展开首帧毫秒级尖峰）
    if let Some(f) = &font {
        f.preheat();
    }
    let addr = config.nas.addr();
    let token = config.nas.token.clone();

    let state = AppState {
        hwnd,
        shared: shared.clone(),
        cmd_tx: cmd_tx.clone(),
        view,
        font,
        pix: tiny_skia::Pixmap::new(CAPSULE_W, CAPSULE_H).unwrap(),
        back_dc: unsafe { CreateCompatibleDC(HDC(null_mut())) },
        hbmp: HBITMAP(null_mut()),
        dib_bits: null_mut(),
        dib_size: (0, 0),
        dragging: false,
        drag_start: (0, 0),
        drag_origin: (0, 0),
        drag_moved: false,
        config,
        settings_hwnd: HWND(null_mut()),
        settings_orig: None,
        settings_card_order: crate::config::DEFAULT_CARD_ORDER,
        settings_font: HFONT(null_mut()),
        last_seq: 0,
        pin_anchor: None,
        pin_hidden: false,
        rings: None,
        acrylic: None,
        seg: None,
        seg_rect: None,
        seg_at: None,
        pin_avoid_cand: None,
        pin_avoid_hits: 0,
        pin_avoid_x: None,
        pin_check_at: None,
        net_peak: (0, 0),
        fs_hidden: false,
    };

    let raw = Box::into_raw(Box::new(state));
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
    }

    // 首次运行或配置不完整：自动打开设置窗，避免用户看到空白胶囊不知所措。
    if unsafe { (*raw).config.nas.needs_setup() } {
        unsafe {
            open_settings(&mut *raw, SettingsAnchor::Panel);
        }
    }

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
    }
    // 启动即按配置位置/方向偏好定好形态：贴左右边缘或显式选了竖向就摆成细长条
    apply_layout(unsafe { &mut *raw }, true);
    paint_frame(unsafe { &mut *raw }, true);

    // 启动时若已开启“固定在任务栏”，立即归位并开始保持置顶
    if unsafe { (*raw).config.capsule.taskbar_pin } {
        apply_taskbar_pin(unsafe { &mut *raw });
    }

    // 全屏应用检测：游戏 / 放映进入全屏时隐藏悬浮窗，退出后恢复。
    // 500ms 一次，每次只是几次窗口查询，空闲开销可忽略。
    unsafe {
        let _ = SetTimer(hwnd, IDT_FS, 500, None);
    }

    // 数据源在窗口首帧完成后再启动：配合入口处的 WSAStartup 串行预热，
    // 双重保证 winsock 首次连接不与窗口/合成器初始化并发（否则系统侧会
    // 出现约 68MB 的异常内存提交，实测概率性触发）。
    let source_handle = WsSource.spawn(SourceCtx {
        hwnd: hwnd.0 as isize,
        state: shared.clone(),
        cmd_rx,
        target: addr,
        token,
    });

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.into() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let _ = cmd_tx.send(SourceCmd::Shutdown);
    let _ = source_handle.join();
    unsafe {
        let _ = timeEndPeriod(1);
    }
}

struct AppState {
    hwnd: HWND,
    shared: Arc<RwLock<SharedState>>,
    cmd_tx: std::sync::mpsc::Sender<SourceCmd>,
    view: Box<dyn View>,
    font: Option<Font>,
    pix: tiny_skia::Pixmap,
    back_dc: HDC,
    hbmp: HBITMAP,
    dib_bits: *mut c_void,
    dib_size: (u32, u32),
    dragging: bool,
    drag_start: (i32, i32),
    /// 按下时光标的屏幕坐标 + 是否已构成拖动（窗口跟手移动时客户区坐标不变，只能用屏幕坐标判定）
    drag_origin: (i32, i32),
    drag_moved: bool,
    config: Config,
    settings_hwnd: HWND,
    /// 设置窗打开时的 (主题, 透明度) 快照：即时预览靠它"取消/关窗"时还原
    settings_orig: Option<(String, f32)>,
    /// 设置窗里卡片的勾选顺序（勾一张挪到末尾），点"确定"才写进配置
    settings_card_order: [u8; crate::config::CARD_COUNT],
    /// 设置窗子控件用的系统 UI 字体（窗口销毁时释放）
    settings_font: HFONT,
    last_seq: u64,
    /// 固定在任务栏时的锚点：(收起态左上角 x, 顶边 y, 是否向上展开)
    pin_anchor: Option<(i32, i32, bool)>,
    /// 固定在任务栏但当前无可用槽位：窗口整体隐藏（托盘仍可操作）
    pin_hidden: bool,
    /// 任务栏内的三环子窗口（固定到任务栏时创建，取消固定时销毁）
    rings: Option<RingsWin>,
    /// 亚克力磨砂背景窗口（底层普通窗口负责系统磨砂 + 圆角裁剪）
    acrylic: Option<AcrylicWin>,
    /// 任务栏可用空段缓存及其计算时刻（像素扫描开销大，降低重算频率）
    seg: Option<(i32, i32)>,
    /// 缓存对应的任务栏矩形（变化时立即失效重算）
    seg_rect: Option<(i32, i32, i32, i32)>,
    seg_at: Option<Instant>,
    /// 遮挡兜底：上次观察到的候选空位（列左缘）与连续命中次数
    pin_avoid_cand: Option<i32>,
    pin_avoid_hits: u8,
    /// 遮挡兜底最终搬到的位置（内存态，不写配置；不再拥挤时回到用户位置）
    pin_avoid_x: Option<i32>,
    /// 遮挡检查的节流时刻（与像素扫描同量级的开销）
    pin_check_at: Option<Instant>,
    /// 网络环兜底刻度（会话峰值 rx/tx，字节/秒）：网卡协商速率读不到时（WiFi 等）用它做满环基准
    net_peak: (u64, u64),
    /// 因全屏应用（游戏 / 放映）而临时隐藏了悬浮窗
    fs_hidden: bool,
}

/// 任务栏内的三环子窗口：归属任务栏（owner = Shell_TrayWnd），常驻其上方。
struct RingsWin {
    hwnd: HWND,
    pix: tiny_skia::Pixmap,
    back_dc: HDC,
    hbmp: HBITMAP,
    dib_bits: *mut c_void,
    dib_size: (u32, u32),
    last_seq: u64,
    dragging: bool,
    drag_moved: bool,
    /// 按下时光标的屏幕坐标（判定点击 vs 拖动）
    drag_origin: (i32, i32),
    /// 光标相对窗口左缘的屏幕坐标偏移
    drag_grab: i32,
    /// 按下瞬间面板是否已展开（抬起时按“按下时的意图”取反，避免失焦收起后又被重新展开）
    expanded_at_press: bool,
}

impl RingsWin {
    fn new(hwnd: HWND) -> Self {
        let (w, h) = (capsule_view::RINGS_W, capsule_view::RINGS_H);
        Self {
            hwnd,
            pix: tiny_skia::Pixmap::new(w, h).unwrap(),
            back_dc: unsafe { CreateCompatibleDC(HDC(null_mut())) },
            hbmp: HBITMAP(null_mut()),
            dib_bits: null_mut(),
            dib_size: (0, 0),
            last_seq: 0,
            dragging: false,
            drag_moved: false,
            drag_origin: (0, 0),
            drag_grab: 0,
            expanded_at_press: false,
        }
    }

    /// 释放 GDI 资源（DestroyWindow 前调用）。
    fn release(&mut self) {
        unsafe {
            if !self.hbmp.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.hbmp.0));
            }
            if !self.back_dc.0.is_null() {
                let _ = DeleteDC(self.back_dc);
            }
        }
        self.hbmp = HBITMAP(null_mut());
        self.back_dc = HDC(null_mut());
        self.dib_bits = null_mut();
        self.dib_size = (0, 0);
    }
}

/// 亚克力磨砂背景窗口：普通窗口（非分层）+ DWM 磨砂 + 圆角裁剪，
/// 位于分层内容窗口之下提供系统级亚克力效果。
struct AcrylicWin {
    hwnd: HWND,
}

impl AcrylicWin {
    fn new(hwnd: HWND) -> Self {
        Self { hwnd }
    }

    /// 释放资源（DestroyWindow 前调用）。亚克力窗口无需 GDI 资源管理。
    fn release(&mut self) {
        // 普通窗口无需手动释放 GDI 资源
    }
}

fn rings_class_name() -> PCWSTR {
    static WIDE: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let v = WIDE.get_or_init(|| "HeTaoCapsule1".encode_utf16().chain(Some(0)).collect());
    PCWSTR(v.as_ptr())
}

fn acrylic_class_name() -> PCWSTR {
    static WIDE: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let v = WIDE.get_or_init(|| "HeTaoCapsule2".encode_utf16().chain(Some(0)).collect());
    PCWSTR(v.as_ptr())
}

/// 亚克力窗口的窗口过程（简化版：仅处理基本消息）。
unsafe extern "system" fn acrylic_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _hdc = BeginPaint(hwnd, &mut ps);
            // 不绘制任何内容：DWM 磨砂背景由系统合成
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_NCHITTEST => {
            // 整个窗口区域对鼠标透明，事件穿透到上层分层窗口
            LRESULT(HTTRANSPARENT as isize)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 创建（如未创建）亚克力磨砂背景窗口。失败返回 false。
fn acrylic_ensure(state: &mut AppState) -> bool {
    if state.acrylic.is_some() {
        return true;
    }
    let instance = unsafe { GetModuleHandleW(None).unwrap_or_default() };

    // 注册亚克力窗口类（首次调用时）
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    REGISTERED.get_or_init(|| {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(acrylic_wndproc),
            hInstance: instance.into(),
            lpszClassName: acrylic_class_name(),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
            hbrBackground: HBRUSH(null_mut()),
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }
        true
    });

    // 创建普通窗口（非分层，用于 DWM 磨砂背景）
    let hwnd = match unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            acrylic_class_name(),
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            capsule_view::RINGS_W as i32,
            capsule_view::RINGS_H as i32,
            None,
            None,
            instance,
            None,
        )
    } {
        Ok(h) => h,
        Err(e) => {
            dbg_log(&format!("acrylic: create failed {e:?}"));
            return false;
        }
    };

    if hwnd.is_invalid() {
        dbg_log("acrylic: create returned null");
        return false;
    }

    // 设置 DWM 系统磨砂背景（亚克力效果）
    unsafe {
        let backdrop_type = 3i32; // DWMSBT_TRANSIENTWINDOW (亚克力)
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &backdrop_type as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );

        // 设置圆角裁剪
        let corner_pref = 2i32; // DWMWCP_ROUND (圆角)
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(33), // DWMWA_WINDOW_CORNER_PREFERENCE
            &corner_pref as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }

    let win = AcrylicWin::new(hwnd);
    state.acrylic = Some(win);
    true
}

/// 销毁亚克力窗口。
fn acrylic_destroy(state: &mut AppState) {
    if let Some(mut a) = state.acrylic.take() {
        a.release();
        unsafe {
            let _ = DestroyWindow(a.hwnd);
        }
    }
}

/// 亚克力窗口定位（ax/ay 为屏幕坐标，aw/ah 为尺寸）。
/// 位置/尺寸未变时不重复 SetWindowPos；可见性由 acrylic_show 单独控制。
fn acrylic_place(state: &mut AppState, ax: i32, ay: i32, aw: i32, ah: i32) {
    if let Some(a) = state.acrylic.as_ref() {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(a.hwnd, &mut rc);
        }
        if rc.left == ax && rc.top == ay && (rc.right - rc.left) == aw && (rc.bottom - rc.top) == ah
        {
            return;
        }
        unsafe {
            // 亚克力窗口在三环/面板之下，Z-order 低于 HWND_TOPMOST
            let _ = SetWindowPos(a.hwnd, HWND_NOTOPMOST, ax, ay, aw, ah, SWP_NOACTIVATE);
        }
    }
}

/// 显示/隐藏亚克力背景层。与面板可见性严格同步：固定态展开时显示、收起时隐藏，
/// 否则收起后会在桌面残留一块无内容的灰色磨砂面板。
fn acrylic_show(state: &AppState, show: bool) {
    if let Some(a) = state.acrylic.as_ref() {
        unsafe {
            let _ = ShowWindow(a.hwnd, if show { SW_SHOWNA } else { SW_HIDE });
        }
    }
}

/// 任务栏窗口句柄（三环子窗口的父窗口）。
fn shell_tray_wnd() -> Option<HWND> {
    let h = unsafe { FindWindowW(static_pcwstr("Shell_TrayWnd"), PCWSTR::null()) }.ok()?;
    (!h.0.is_null()).then_some(h)
}

/// 创建（如未创建）任务栏内的三环子窗口。任务栏不存在或创建失败返回 false。
fn rings_ensure(state: &mut AppState) -> bool {
    if state.rings.is_some() {
        return true;
    }
    let Some(tb) = shell_tray_wnd() else {
        return false;
    };
    let instance = unsafe { GetModuleHandleW(None).unwrap_or_default() };
    // 三环窗口建成任务栏的“从属窗口”（owner = Shell_TrayWnd）：
    // 顶层分层窗口（UpdateLayeredWindow 上屏可用），归属任务栏后始终压在其上、
    // 随任务栏一起隐藏，点任务栏也不会被盖住。跨进程 WS_CHILD 子窗口无法分层合成，故不采用。
    let hwnd = match unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            rings_class_name(),
            window_title(),
            WS_POPUP,
            0,
            0,
            capsule_view::RINGS_W as i32,
            capsule_view::RINGS_H as i32,
            None,
            None,
            instance,
            None,
        )
    } {
        Ok(h) => h,
        Err(e) => {
            dbg_log(&format!("rings: create failed {e:?}"));
            return false;
        }
    };
    if hwnd.is_invalid() {
        dbg_log("rings: create returned null");
        return false;
    }
    unsafe {
        // GWL_HWNDPARENT 对顶层窗口即设置 owner
        SetWindowLongPtrW(hwnd, GWL_HWNDPARENT, tb.0 as isize);
    }
    let win = RingsWin::new(hwnd);
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as *mut AppState as isize);
    }
    state.rings = Some(win);

    // 同步创建亚克力磨砂背景窗口（底层）
    acrylic_ensure(state);

    true
}

/// 销毁三环子窗口（取消固定到任务栏时）。
fn rings_destroy(state: &mut AppState) {
    // 先销毁亚克力背景窗口
    acrylic_destroy(state);

    if let Some(mut r) = state.rings.take() {
        r.release();
        unsafe {
            let _ = DestroyWindow(r.hwnd);
        }
    }
}

/// 三环窗口定位（ax/ay 为屏幕坐标）。归属任务栏的顶层窗口，直接用屏幕坐标。
/// 位置未变时不重复 SetWindowPos（避免周期重排带来的额外开销）。
fn rings_place(state: &mut AppState, ax: i32, ay: i32) {
    if let Some(r) = state.rings.as_mut() {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(r.hwnd, &mut rc);
        }
        let visible = unsafe { IsWindowVisible(r.hwnd) }.as_bool();
        if visible && rc.left == ax && rc.top == ay {
            return;
        }
        unsafe {
            let _ = SetWindowPos(
                r.hwnd,
                HWND_TOPMOST,
                ax,
                ay,
                capsule_view::RINGS_W as i32,
                capsule_view::RINGS_H as i32,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }
}

/// 渲染并上屏三环子窗口。
fn rings_paint(state: &mut AppState, force: bool) {
    let Some(r) = state.rings.as_mut() else {
        return;
    };
    let seq = match state.shared.read() {
        Ok(g) => g.seq(),
        Err(e) => {
            eprintln!("[App] RwLock poisoned in rings_paint seq check: {:?}", e);
            return;
        }
    };
    if !force && seq == r.last_seq {
        return;
    }
    r.last_seq = seq;
    let frame_data = match state.shared.read() {
        Ok(g) => g.grab(),
        Err(e) => {
            eprintln!("[App] RwLock poisoned in rings_paint grab: {:?}", e);
            return;
        }
    };
    let frame = frame_data.as_frame();
    let (w, h) = (capsule_view::RINGS_W, capsule_view::RINGS_H);
    // 整块铺一层 alpha=1 的近乎透明底：分层窗口只有 alpha>0 的像素才响应鼠标，
    // 否则只有细环与文字可点、点环之间/环内的空处会穿透到任务栏，感觉"点不动"。
    let mut hit_bg = tiny_skia::Paint::default();
    hit_bg.set_color_rgba8(0, 0, 0, 1);
    hit_bg.blend_mode = tiny_skia::BlendMode::Source;
    r.pix.fill_rect(
        tiny_skia::Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap(),
        &hit_bg,
        tiny_skia::Transform::identity(),
        None,
    );
    // 协商速率读不到时（WiFi 网卡无 sysfs speed）网络环改用会话峰值做满环基准，
    // 峰值按帧缓慢衰减，避免历史尖峰长期钉住刻度
    let (rx, tx) = capsule_view::net_io_sum(frame.snap);
    let decay = |p: u64, v: u64| (p * 99 / 100).max(v);
    state.net_peak = (decay(state.net_peak.0, rx), decay(state.net_peak.1, tx));
    capsule_view::draw_rings(&mut r.pix, &frame, state.font.as_ref(), state.net_peak);
    let opacity = state.config.capsule.opacity.clamp(0.05, 1.0);
    blit_and_present(
        r.hwnd,
        &mut r.pix,
        r.back_dc,
        &mut r.hbmp,
        &mut r.dib_bits,
        &mut r.dib_size,
        opacity,
        &[],
        h,
        (w, h),
    );
}

/// 三环窗口在任务栏内的纵向基准（屏幕坐标）：条内垂直居中。
fn ring_anchor_y() -> i32 {
    match taskbar_rect() {
        Some(t) => t.top + (t.bottom - t.top - capsule_view::RINGS_H as i32) / 2,
        None => 0,
    }
}

/// 三环窗口拖动跟手：在整个任务栏范围内移动（不再限制在扫描出的空段里）。
fn rings_drag_move(state: &mut AppState) {
    let Some(tr) = taskbar_rect() else { return };
    let Some((x0, x1)) = taskbar_span(&tr) else {
        return;
    };
    let Some(grab) = state.rings.as_ref().map(|r| r.drag_grab) else {
        return;
    };
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    let ax = pin_fit(x0, x1, Some(pt.x - grab));
    state.pin_anchor = Some((ax, ring_anchor_y(), true));
    rings_place(state, ax, ring_anchor_y());
}

/// 三环窗口拖动松手：记住选定位置。
fn rings_drag_commit(state: &mut AppState) {
    let Some(tr) = taskbar_rect() else { return };
    let Some((x0, x1)) = taskbar_span(&tr) else {
        return;
    };
    let Some(r) = state.rings.as_ref() else {
        return;
    };
    let mut wr = RECT::default();
    unsafe {
        let _ = GetWindowRect(r.hwnd, &mut wr);
    }
    let ax = pin_fit(x0, x1, Some(wr.left));
    state.config.capsule.taskbar_x = Some(ax);
    let _ = crate::config::save(&state.config);
    state.pin_anchor = Some((ax, ring_anchor_y(), true));
    rings_place(state, ax, ring_anchor_y());
}

/// 任务栏内三环子窗口的消息处理：点击展开/收起面板，拖动在任务栏内换位。
unsafe extern "system" fn rings_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if ptr == 0 {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *(ptr as *mut AppState);

    match msg {
        WM_APP_DATA | WM_APP_PANEL => {
            rings_paint(state, true);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let mut pt = POINT::default();
            let mut wr = RECT::default();
            unsafe {
                let _ = GetCursorPos(&mut pt);
                let _ = GetWindowRect(hwnd, &mut wr);
            }
            let expanded_now = state.view.expanded();
            if let Some(r) = state.rings.as_mut() {
                r.dragging = true;
                r.drag_moved = false;
                r.drag_origin = (pt.x, pt.y);
                r.drag_grab = pt.x - wr.left;
                r.expanded_at_press = expanded_now;
            }
            unsafe {
                let _ = SetCapture(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let moved = match state.rings.as_mut() {
                Some(r) if r.dragging => {
                    let mut pt = POINT::default();
                    unsafe {
                        let _ = GetCursorPos(&mut pt);
                    }
                    if (pt.x - r.drag_origin.0).abs() >= 4 || (pt.y - r.drag_origin.1).abs() >= 4 {
                        r.drag_moved = true;
                    }
                    r.drag_moved
                }
                _ => false,
            };
            if moved {
                rings_drag_move(state);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (dragging, moved, was_expanded) = match state.rings.as_mut() {
                Some(r) => {
                    let d = r.dragging;
                    let m = r.drag_moved;
                    let e = r.expanded_at_press;
                    r.dragging = false;
                    (d, m, e)
                }
                None => (false, false, false),
            };
            unsafe {
                let _ = ReleaseCapture();
            }
            if dragging {
                if moved {
                    rings_drag_commit(state);
                } else {
                    // 按「按下时的意图」取反：三环窗口在任务栏内，按下会先让面板失焦收起
                    set_panel_open(state, !was_expanded);
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            show_tray_menu(state);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn create_window(instance: HMODULE, size: (u32, u32)) -> HWND {
    let ex_style = WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TOOLWINDOW;
    let style = WS_POPUP;
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            class_name(),
            window_title(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            size.0 as i32,
            size.1 as i32,
            None,
            None,
            instance,
            None,
        )
        .unwrap_or_default()
    };
    MAIN_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
    hwnd
}

/// 启动时的收起态宽度：固定在任务栏用紧凑宽度，竖排用细长条宽度，否则为药丸胶囊宽度。
fn initial_w(config: &Config) -> u32 {
    if config.capsule.taskbar_pin {
        CAPSULE_W
    } else if config.capsule.orientation_hint() == Some(true) {
        capsule_view::PILL_V_W
    } else {
        CapsuleView::pill_width()
    }
}

/// 未固定时的收起态尺寸：竖排＝细长条，横排＝药丸胶囊。
fn collapsed_main_size(vertical: bool) -> (u32, u32) {
    if vertical {
        (capsule_view::PILL_V_W, capsule_view::PILL_V_H)
    } else {
        (CapsuleView::pill_width(), CAPSULE_H)
    }
}

/// 收起态默认位置：
/// - 横排：吸附工作区顶边，横向让开标题栏的窗口按钮区（最大化窗口的三键就在右上角）；
/// - 竖排：吸附工作区右缘，纵向让开标题栏那一行。
/// SM_CXSIZE 在不同系统上口径不一，按钮区宽度取 150px 兜底。
fn default_capsule_pos(w: i32, vertical: bool) -> (i32, i32) {
    /// 与屏幕边缘、与按钮区之间的留白
    const MARGIN: i32 = 12;
    let mut rc = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rc as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    // 标题栏高度（随系统 DPI 缩放）
    let caption = unsafe { GetSystemMetrics(SM_CYCAPTION) };
    if vertical {
        (rc.right - w, rc.top + caption + MARGIN)
    } else {
        let buttons_w =
            (unsafe { GetSystemMetrics(SM_CXSIZE) * 3 + GetSystemMetrics(SM_CXPADDEDBORDER) * 2 })
                .max(150);
        ((rc.right - buttons_w - w - MARGIN).max(rc.left), rc.top)
    }
}

/// 判定当前应采用的方向与展开侧：
/// - 显式选了 横向/竖向 就照用（竖排的"面板在哪一侧"取离窗口较近的那边）；
/// - `auto` 且 `from_pos`（启动/拖动落定）时：贴左右边缘 12px 内 → 竖排，贴哪边决定面板在另一侧；
/// - 否则横排。
fn desired_layout(state: &AppState, from_pos: bool) -> (bool, bool) {
    // 固定到任务栏时主窗口只作展开面板用，方向设置不参与
    if state.config.capsule.taskbar_pin {
        return (false, false);
    }
    let mut rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(state.hwnd, &mut rc);
    }
    let wa = monitor_work_area(unsafe { MonitorFromWindow(state.hwnd, MONITOR_DEFAULTTONEAREST) });
    if let Some(v) = state.config.capsule.orientation_hint() {
        // 竖排：贴右缘时面板在左（窗口右缘固定），贴左缘时面板在右
        let panel_left = (wa.right - rc.right) <= (rc.left - wa.left);
        return (v, v && panel_left);
    }
    if from_pos {
        // 展开态（尤其超大卡片贴边）窗口矩形是面板尺寸，直接拿它判「贴边」会把横排
        // 误判成竖排（收起后胶囊变成顶部竖三环）；此时改用配置里记录的收起态落点判断。
        let (left, right) = if state.view.expanded() {
            let w = CapsuleView::pill_width() as i32;
            match state.config.capsule.x {
                Some(x) => (x, x + w),
                None => (rc.left, rc.right),
            }
        } else {
            (rc.left, rc.right)
        };
        if left <= wa.left + SNAP_PX {
            return (true, false);
        }
        if right >= wa.right - SNAP_PX {
            return (true, true);
        }
    }
    (false, false)
}

/// 把方向/展开侧同步给渲染侧；有变化时立刻重排窗口并重绘（收起态尺寸也随方向变化）。
fn apply_layout(state: &mut AppState, from_pos: bool) {
    let (vertical, panel_left) = desired_layout(state, from_pos);
    let changed = {
        let mut sh = match state.shared.write() {
            Ok(g) => g,
            Err(e) => {
                eprintln!("[App] RwLock poisoned in apply_layout: {:?}", e);
                return;
            }
        };
        if sh.vertical == vertical && sh.panel_left == panel_left {
            false
        } else {
            sh.vertical = vertical;
            sh.panel_left = panel_left;
            sh.bump_seq();
            true
        }
    };
    if changed {
        resize_window(state);
        paint_frame(state, true);
    }
}

/// 初始位置：配置优先，否则用默认落点（按方向偏好选横排/竖排）。
fn set_initial_pos(hwnd: HWND, config: &Config) {
    let vertical = config.capsule.orientation_hint() == Some(true);
    let (cw, _) = collapsed_main_size(vertical);
    let (mut x, mut y) = match (config.capsule.x, config.capsule.y) {
        (Some(x), Some(y)) => (x, y),
        _ => default_capsule_pos(cw as i32, vertical),
    };
    // 落点必须在某台显示器内。不能只看 x<0/y<0——主屏左侧或上方的副屏坐标本来就是负的，
    // 那样会把副屏上的落点强行拉到主屏左上角（表现为"胶囊莫名跳到另一块屏"）。
    let pt = POINT { x, y };
    if unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONULL) }.is_invalid() {
        let (dx, dy) = default_capsule_pos(cw as i32, vertical);
        x = dx;
        y = dy;
    }
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// 登记托盘图标。返回 false 表示当时 shell 不可用（例如开机自启早于资源管理器就绪）。
fn tray_init(hwnd: HWND, icon: HICON) -> bool {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    let tip: Vec<u16> = "核桃胶囊".encode_utf16().collect();
    let len = tip.len().min(nid.szTip.len() - 1);
    nid.szTip[..len].copy_from_slice(&tip[..len]);
    // 图标已存在时 NIM_ADD 会失败（例如 shell 反复广播 TaskbarCreated），回退到 NIM_MODIFY
    let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &nid).as_bool() }
        || unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() };
    if !ok {
        dbg_log("tray: NIM_ADD 失败");
    }
    ok
}

/// 资源管理器重启时 shell 会向所有顶层窗口广播这条注册消息（"TaskbarCreated"）。
fn taskbar_created_msg() -> u32 {
    static MSG: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *MSG.get_or_init(|| {
        let name: Vec<u16> = "TaskbarCreated".encode_utf16().chain(Some(0)).collect();
        unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) }
    })
}

/// 资源管理器重启后的恢复：托盘图标已被 shell 清空，必须重新登记，否则只能等进程重启；
/// 三环是旧 Shell_TrayWnd 的从属窗口，重建后重新绑到新任务栏。
fn rebuild_after_shell_restart(state: &mut AppState) {
    let _ = tray_init(state.hwnd, app_icons().1);
    if !state.config.capsule.taskbar_pin {
        return;
    }
    // 旧任务栏随 shell 一起销毁后，三环窗口即使句柄还在也已失去 owner：
    // 不再压在任务栏之上、也不再随任务栏隐藏，表现为"窗口存在但画不出来"。
    // 所以不能靠 IsWindow 判断存活，一律重建并绑到新任务栏。
    let old_pos = state.rings.as_ref().map(|r| {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(r.hwnd, &mut rc);
        }
        (rc.left, rc.top)
    });
    if let Some(mut r) = state.rings.take() {
        r.release();
        unsafe {
            let _ = DestroyWindow(r.hwnd);
        }
    }
    // 位置与任务栏扫描缓存一并清空，交给 sync_taskbar_state 重新选位
    state.pin_anchor = None;
    state.pin_hidden = false;
    state.seg = None;
    state.seg_rect = None;
    state.seg_at = None;
    state.pin_avoid_cand = None;
    state.pin_avoid_hits = 0;
    state.pin_avoid_x = None;
    state.pin_check_at = None;
    if !rings_ensure(state) {
        return;
    }
    if state.view.expanded() {
        // 面板展开着：sync_taskbar_state 会跳过重算，先沿用重建前的位置
        if let Some((x, y)) = old_pos {
            rings_place(state, x, y);
        }
    } else {
        sync_taskbar_state(state);
    }
    rings_paint(state, true);
}

fn tray_remove(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

const WM_APP_TRAY: u32 = WM_APP + 2;
/// 面板展开期间，鼠标在面板外按下（由低级鼠标钩子投递）。
/// 注意避开已在用的 WM_APP+1（数据）、+2（托盘回调）、+3（面板就绪）。
const WM_APP_CLICK_OUTSIDE: u32 = WM_APP + 4;
/// 展开期间安装的全局低级鼠标钩子句柄；0 表示未安装
static MOUSE_HOOK: AtomicIsize = AtomicIsize::new(0);
/// 主窗口句柄：钩子回调在消息泵上下文里执行，投递消息时也要能拿到它
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
const ID_TRAY_SETTINGS: u32 = 1001;
const ID_TRAY_EXIT: u32 = 1002;
/// 托盘菜单「胶囊 / 三环 切换」（勾选项，反映当前是否挂在任务栏）
const ID_TRAY_PIN: u32 = 1003;
/// 展开/收起动效定时器 ID
const IDT_ANIM: usize = 0x4A11;
/// 固定在任务栏时，周期性把窗口重新提到任务栏之上的定时器 ID
const IDT_PIN: usize = 0x4A12;
/// 全屏应用检测定时器 ID（500ms 一次，游戏进出全屏时及时隐藏/恢复）
const IDT_FS: usize = 0x4A13;
/// 设置窗可选的透明度档位（同时也是设置窗单选钮的顺序）
const OPACITY_STEPS: [f32; 4] = [1.0, 0.85, 0.70, 0.55];

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if ptr == 0 {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *(ptr as *mut AppState);

    match msg {
        WM_APP_DATA => {
            paint_frame(state, false);
            // 固定在任务栏时，收起态三环由任务栏内的子窗口显示，随数据同步刷新
            rings_paint(state, false);
            LRESULT(0)
        }
        WM_APP_PANEL => {
            // 后台面板构建完成：seq 与上帧相同（数据未变、仅面板就绪），
            // 必须强制重绘一帧，否则会被 seq 去重早退、新面板要等下一秒数据帧才上屏。
            // 面板高度可能随设备增减变化，固定态展开时同步窗口高度（几何未变会被跳过）。
            if state.config.capsule.taskbar_pin && state.view.expanded() {
                resize_window(state);
            }
            paint_frame(state, true);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == IDT_ANIM => {
            let more = state.view.tick();
            resize_window(state);
            paint_frame(state, true);
            if !more {
                unsafe {
                    let _ = KillTimer(hwnd, IDT_ANIM);
                }
                // 固定到任务栏：收起动画结束后面板直接消失，收起态由任务栏内三环窗口承担
                if state.config.capsule.taskbar_pin && !state.view.expanded() {
                    unsafe {
                        let _ = ShowWindow(state.hwnd, SW_HIDE);
                    }
                    acrylic_show(state, false);
                } else if !state.view.expanded() {
                    // 收起完成：回到配置里记的收起态落点。展开时若为让长面板进屏而临时上移，
                    // 这个位移不该被带进收起态（竖排 838 高的面板在矮屏上会触发）。
                    if let (Some(cx), Some(cy)) = (state.config.capsule.x, state.config.capsule.y) {
                        let mut rc = RECT::default();
                        unsafe {
                            let _ = GetWindowRect(state.hwnd, &mut rc);
                        }
                        if rc.left != cx || rc.top != cy {
                            unsafe {
                                let _ = SetWindowPos(
                                    state.hwnd,
                                    None,
                                    cx,
                                    cy,
                                    0,
                                    0,
                                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                                );
                            }
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == IDT_PIN => {
            // 周期重估任务栏空间：任务栏内容变化（天气/小组件开关）时重新选空段并归位三环。
            if state.config.capsule.taskbar_pin {
                sync_taskbar_state(state);
            } else {
                unsafe {
                    let _ = KillTimer(hwnd, IDT_PIN);
                }
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == IDT_FS => {
            sync_fullscreen_state(state);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = lparam.0 as i16 as i32;
            let y = (lparam.0 >> 16) as i16 as i32;
            state.dragging = true;
            state.drag_start = (x, y);
            state.drag_moved = false;
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            state.drag_origin = (pt.x, pt.y);
            let _ = SetCapture(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            // 大卡网络柱图悬停：柱子下标变化时重绘一次（不是每帧）
            if state.view.expanded() {
                let hx = lparam.0 as i16 as i32;
                let hy = (lparam.0 >> 16) as i16 as i32;
                if state.view.set_hover_bar(state.view.hit_bar(hx, hy)) {
                    paint_frame(state, true);
                }
                // 登记离开通知，鼠标移出窗口时收到 WM_MOUSELEAVE
                unsafe {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                }
            }
            // 固定到任务栏时主窗口只作展开面板显示，不参与拖动（收起态由任务栏内三环窗口拖动换位）
            if state.config.capsule.taskbar_pin {
                return LRESULT(0);
            }
            if state.dragging {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                if (pt.x - state.drag_origin.0).abs() >= 4
                    || (pt.y - state.drag_origin.1).abs() >= 4
                {
                    state.drag_moved = true;
                }
                // 未固定：自由拖动
                let x = lparam.0 as i16 as i32;
                let y = (lparam.0 >> 16) as i16 as i32;
                let mut rc = RECT::default();
                let _ = GetWindowRect(hwnd, &mut rc);
                let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
                let mut nx = rc.left + (x - state.drag_start.0);
                let mut ny = rc.top + (y - state.drag_start.1);
                snap_to_work_area(hwnd, w, h, &mut nx, &mut ny);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    nx,
                    ny,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            // 鼠标离开窗口：清掉柱图悬停高亮
            if state.view.set_hover_bar(None) {
                paint_frame(state, true);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if state.dragging {
                state.dragging = false;
                let _ = ReleaseCapture();
                let x = lparam.0 as i16 as i32;
                let y = (lparam.0 >> 16) as i16 as i32;
                if !state.drag_moved {
                    // 展开态点中顶栏设置齿轮 → 打开设置窗口，不收起胶囊
                    if state.view.hit_settings(x, y) {
                        open_settings(state, SettingsAnchor::Panel);
                    } else if !state.view.expanded() || click_on_capsule_bar(state, x, y) {
                        // 收起态点胶囊 → 展开；展开态只有点到胶囊条（竖条 / 顶栏）才收起。
                        // 点在面板正文（大卡尤甚）不做任何事，收起只交给「点面板外」钩子，
                        // 否则齿轮命中稍有偏差就会把面板关掉。
                        toggle_expand(state);
                    }
                } else if !state.config.capsule.taskbar_pin && !state.view.expanded() {
                    // 收起态拖动落定：记住位置（下次启动回到这里），
                    // 并让"自动"档按新位置判定横排/竖排（贴到左右边缘即变竖条）
                    let mut rc = RECT::default();
                    let _ = GetWindowRect(state.hwnd, &mut rc);
                    state.config.capsule.x = Some(rc.left);
                    state.config.capsule.y = Some(rc.top);
                    let _ = crate::config::save(&state.config);
                    apply_layout(state, true);
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u32 == VK_ESCAPE.0 as u32 {
                collapse(state);
            }
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            dbg_log("WM_KILLFOCUS");
            // 设置窗口打开时它才是前台，不因此收起
            if state.view.expanded() && wparam.0 as isize != state.settings_hwnd.0 as isize {
                // 固定到任务栏时，鼠标移向任务栏/三环也会让面板失焦——此时不能收起，
                // 否则用户还没点到三环面板就没了（点三环收起也因此失效）。
                let over_own = cursor_over_main(state)
                    || state
                        .rings
                        .as_ref()
                        .is_some_and(|r| cursor_over_window(r.hwnd));
                if !over_own {
                    collapse(state);
                }
            }
            LRESULT(0)
        }
        WM_APP_CLICK_OUTSIDE => {
            // 面板展开时，点击自身窗口（面板/三环）以外的任意位置即收起。
            // 用全局鼠标钩子而不是窗口焦点：面板是置顶浮窗，经常拿不到前台焦点，
            // 只靠 WM_KILLFOCUS 会出现"点了桌面面板不收"的情况。
            // 设置窗口开着时不收起，避免调设置时面板闪没。
            if state.config.capsule.click_outside_collapse
                && state.view.expanded()
                && state.settings_hwnd.0.is_null()
            {
                let over_own = cursor_over_main(state)
                    || state
                        .rings
                        .as_ref()
                        .is_some_and(|r| cursor_over_window(r.hwnd));
                if !over_own {
                    collapse(state);
                }
            }
            LRESULT(0)
        }
        WM_APP_TRAY => {
            if lparam.0 as u32 == WM_RBUTTONUP {
                show_tray_menu(state);
            }
            LRESULT(0)
        }
        m if m == taskbar_created_msg() => {
            // 资源管理器重启：shell 会清空整个通知区域，托盘图标必须重新登记
            dbg_log("TaskbarCreated");
            rebuild_after_shell_restart(state);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 as u32;
            if id == ID_TRAY_EXIT {
                let _ = state.cmd_tx.send(SourceCmd::Shutdown);
                tray_remove(state.hwnd);
                PostQuitMessage(0);
            } else if id == ID_TRAY_SETTINGS {
                // 从托盘菜单打开：在光标（托盘图标）处弹出
                open_settings(state, SettingsAnchor::Cursor);
            } else if id == ID_TRAY_PIN {
                // 胶囊 ↔ 三环 切换：与设置窗里那个开关走同一套动作
                state.config.capsule.taskbar_pin = !state.config.capsule.taskbar_pin;
                let _ = crate::config::save(&state.config);
                // 取消固定时 apply_taskbar_pin 自己会再存一次（它还会落默认位置），
                // 固定方向上不会存，所以这里统一先存。
                apply_taskbar_pin(state);
            }
            LRESULT(0)
        }
        WM_DESTROY | WM_CLOSE => {
            dbg_log("WM_DESTROY/CLOSE");
            let _ = state.cmd_tx.send(SourceCmd::Shutdown);
            tray_remove(state.hwnd);
            rings_destroy(state);
            // 清理全局鼠标钩子（规范做法：虽然进程结束时系统会自动清理，但应主动释放）
            mouse_hook_remove();
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 点击是否落在「可收起的胶囊条」上：竖排是窗口另一侧的竖条，横排是顶部那条胶囊。
/// 超大卡片的窗口本身就是整块面板；固定到任务栏时收起由三环负责——两种情况都返回 false，
/// 即展开态点面板正文不会收起。
fn click_on_capsule_bar(state: &AppState, x: i32, y: i32) -> bool {
    if state.config.capsule.expand_big() || state.config.capsule.taskbar_pin {
        return false;
    }
    let sh = match state.shared.read() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[App] RwLock poisoned in click_on_capsule_bar: {:?}", e);
            return false;
        }
    };
    if sh.vertical {
        let bar_x0 = if sh.panel_left {
            capsule_view::PANEL_W as i32
        } else {
            0
        };
        x >= bar_x0 && x < bar_x0 + capsule_view::PILL_V_W as i32
    } else {
        y < CAPSULE_H as i32
    }
}

/// 光标是否落在主窗口的**可见内容**上。
/// 竖排展开时窗口右侧（或左侧）有一条只放竖条的透明空带，点在空带上应视为"面板外"，
/// 否则点面板旁边的空白收不起面板。
fn cursor_over_main(state: &AppState) -> bool {
    let mut pt = POINT::default();
    let mut rc = RECT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
        if GetWindowRect(state.hwnd, &mut rc).is_err() {
            return false;
        }
    }
    if pt.x < rc.left || pt.x >= rc.right || pt.y < rc.top || pt.y >= rc.bottom {
        return false;
    }
    let sh = match state.shared.read() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[App] RwLock poisoned in cursor_over_main: {:?}", e);
            return false;
        }
    };
    if !(sh.vertical && state.view.expanded()) {
        return true;
    }
    let (x, y) = (pt.x - rc.left, pt.y - rc.top);
    let panel_x0 = if sh.panel_left {
        0
    } else {
        capsule_view::PILL_V_W as i32
    };
    let bar_x0 = if sh.panel_left {
        capsule_view::PANEL_W as i32
    } else {
        0
    };
    let on_panel = x >= panel_x0 && x < panel_x0 + capsule_view::PANEL_W as i32;
    let on_bar = x >= bar_x0
        && x < bar_x0 + capsule_view::PILL_V_W as i32
        && y < capsule_view::PILL_V_H as i32;
    on_panel || on_bar
}

/// 光标当前是否落在指定窗口矩形内。
fn cursor_over_window(h: HWND) -> bool {
    if h.0.is_null() {
        return false;
    }
    let mut pt = POINT::default();
    let mut rc = RECT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
        if GetWindowRect(h, &mut rc).is_err() {
            return false;
        }
    }
    pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom
}

/// 安装全局低级鼠标钩子（仅在面板展开期间存在，收起立即卸载，常驻零开销）。
fn mouse_hook_install() {
    if MOUSE_HOOK.load(Ordering::Relaxed) != 0 {
        return;
    }
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        if let Ok(h) = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), instance, 0) {
            MOUSE_HOOK.store(h.0 as isize, Ordering::Relaxed);
        }
    }
}

fn mouse_hook_remove() {
    let v = MOUSE_HOOK.swap(0, Ordering::Relaxed);
    if v != 0 {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(v as *mut c_void));
        }
    }
}

/// 低级鼠标钩子回调：钩子在安装它的线程（本进程消息泵）里被调用，
/// 因此这里只做最轻的事——把「按钮按下」投给主窗口，是否在面板外、要不要收起
/// 都交给 wndproc 判定，避免在钩子里做窗口操作拖慢全局鼠标输入。
unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let m = wparam.0 as u32;
        if m == WM_LBUTTONDOWN || m == WM_RBUTTONDOWN || m == WM_MBUTTONDOWN {
            let h = MAIN_HWND.load(Ordering::Relaxed);
            if h != 0 {
                let _ = PostMessageW(
                    HWND(h as *mut c_void),
                    WM_APP_CLICK_OUTSIDE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

fn collapse(state: &mut AppState) {
    if !state.view.expanded() {
        return;
    }
    mouse_hook_remove(); // 面板收起，撤掉"点外部收起"用的全局鼠标钩子
    dbg_log("collapse");
    state.view.set_expanded(false);
    let _ = state.cmd_tx.send(SourceCmd::SetDetail(Detail::Collapsed));
    // 收起动画（固定到任务栏时从下往上缩回任务栏）；动画结束由 IDT_ANIM 收尾隐藏窗口
    start_anim(state);
}

fn paint_frame(state: &mut AppState, force: bool) {
    // 固定到任务栏但无可用槽位时窗口已隐藏，无需上屏
    if state.pin_hidden {
        return;
    }
    // 固定到任务栏的收起态由任务栏内的三环子窗口显示，主窗口此时隐藏
    if state.config.capsule.taskbar_pin && !state.view.expanded() {
        return;
    }
    let seq = match state.shared.read() {
        Ok(g) => g.seq(),
        Err(e) => {
            eprintln!("[App] RwLock poisoned in paint_frame seq check: {:?}", e);
            return;
        }
    };
    if !force && seq == state.last_seq {
        return;
    }
    state.last_seq = seq;

    let frame_data = match state.shared.read() {
        Ok(g) => g.grab(),
        Err(e) => {
            eprintln!("[App] RwLock poisoned in paint_frame grab: {:?}", e);
            return;
        }
    };
    // 超大卡片：几何按当前显示器工作区算好下发给视图（面板大小恒定，与卡片数无关）
    if state.config.capsule.expand_big() {
        let g = big_geometry(state);
        state.view.set_big_geometry(g.0, g.1, g.2);
    }
    let (vis_w, vis_h) = state.view.frame_size(&frame_data.as_frame());
    let (buf_w, buf_h) = state.view.buffer_size(&frame_data.as_frame());

    // 像素缓冲与 DIB 按“缓冲尺寸”分配：展开动画期间尺寸恒定为全展开尺寸，
    // 只在收起↔展开切换时各重建一次，避免动画每帧分配像素缓冲、重建 GDI DIB。
    if state.pix.width() != buf_w || state.pix.height() != buf_h {
        state.pix = tiny_skia::Pixmap::new(buf_w, buf_h).unwrap();
        unsafe {
            if !state.hbmp.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(state.hbmp.0));
            }
        }
        state.hbmp = HBITMAP(null_mut());
        state.dib_bits = null_mut();
        state.dib_size = (0, 0);
    }

    // 只清可视矩形（缓冲可能高于/宽于可视区，动画期外沿不可见）。
    let mut clear = tiny_skia::Paint::default();
    clear.set_color_rgba8(0, 0, 0, 0);
    clear.blend_mode = tiny_skia::BlendMode::Source;
    state.pix.fill_rect(
        tiny_skia::Rect::from_xywh(0.0, 0.0, vis_w as f32, vis_h as f32).unwrap(),
        &clear,
        tiny_skia::Transform::identity(),
        None,
    );
    let presented = state.view.render(
        &mut state.pix,
        &frame_data.as_frame(),
        state.font.as_ref(),
        (vis_w, vis_h),
    );
    if !presented {
        // 视图判定本帧画面无变化（稳态数据帧已派发后台构建，等 WM_APP_PANEL
        // 携带新面板再上屏）：跳过像素拷贝与 UpdateLayeredWindow 提交。
        return;
    }

    // 暗色主题：整套界面按亮色构建，上屏前一次性把中性色翻转（彩色状态色原样保留）
    if frame_data.dark {
        capsule_view::apply_dark(&mut state.pix);
    }

    let opacity = state.config.capsule.opacity.clamp(0.05, 1.0);
    let fade = state.view.row_fade();
    blit_and_present(
        state.hwnd,
        &mut state.pix,
        state.back_dc,
        &mut state.hbmp,
        &mut state.dib_bits,
        &mut state.dib_size,
        opacity,
        fade,
        buf_h,
        (vis_w, vis_h),
    );
}

/// 把已渲染好的预乘 RGBA 像素缓冲按整体不透明度与逐行淡入处理后，
/// 完成 RGBA→BGRA 交换，经 GDI DIB + UpdateLayeredWindow 上屏。
/// 任务栏内三环子窗口与主窗口共用本函数。
#[allow(clippy::too_many_arguments)]
fn blit_and_present(
    hwnd: HWND,
    pix: &mut tiny_skia::Pixmap,
    back_dc: HDC,
    hbmp: &mut HBITMAP,
    dib_bits: &mut *mut c_void,
    dib_size: &mut (u32, u32),
    opacity: f32,
    fade: &[u8],
    buf_h: u32,
    vis: (u32, u32),
) {
    let (vis_w, vis_h) = vis;
    let buf_w = pix.width();
    let buf_stride = buf_w as usize * 4;
    let vis_row_bytes = vis_w as usize * 4;
    // 乘法用按行惰性构建的 256 项查找表：全帧行不透明度只有少数几种
    // （顶栏/面板），避免每像素数百万次 f32↔u8 转换。
    let data = pix.data_mut();
    let mut lut_key: i32 = -1;
    let mut lut = [0u8; 256];
    for y in 0..vis_h as usize {
        let f = fade.get(y).copied().unwrap_or(255) as i32;
        let start = y * buf_stride;
        let row = &mut data[start..start + vis_row_bytes];
        if f == 255 && opacity >= 0.9999 {
            for px4 in row.as_chunks_mut::<4>().0 {
                px4.swap(0, 2);
            }
        } else {
            if f != lut_key {
                lut_key = f;
                let k = opacity * f as f32 / 255.0;
                for (v, e) in lut.iter_mut().enumerate() {
                    // 源 alpha 非 0 的像素，缩放后**至少保留 1**：分层窗口的命中测试按 alpha
                    // 判定，乘完透明度取整到 0 会让这块区域变成"鼠标穿透"。三环窗口整块铺的
                    // alpha=1 近透明底就是这么被 70%/55% 透明度抹掉的——表现为"必须点在三环的
                    // 数值上才能弹出面板"；保留到 1 则与 100% 不透明度时的可点区域完全一致，
                    // 视觉上 0→1 无差别。源 alpha=0 的区域仍然保持穿透。
                    *e = if v == 0 {
                        0
                    } else {
                        (v as f32 * k).max(1.0) as u8
                    };
                }
            }
            for px4 in row.as_chunks_mut::<4>().0 {
                px4[0] = lut[px4[0] as usize];
                px4[1] = lut[px4[1] as usize];
                px4[2] = lut[px4[2] as usize];
                px4[3] = lut[px4[3] as usize];
                px4.swap(0, 2);
            }
        }
    }

    ensure_dib(back_dc, hbmp, dib_bits, dib_size, buf_w, buf_h);

    // DIB 与像素缓冲等宽（stride 相同），逐行拷贝可视区域到 DIB 左上矩形。
    unsafe {
        let src = pix.data().as_ptr();
        let dst = *dib_bits as *const u8;
        for y in 0..vis_h as usize {
            let off = y * buf_stride;
            std::ptr::copy_nonoverlapping(src.add(off), dst.add(off) as *mut u8, vis_row_bytes);
        }
    }

    let mut rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rc);
    }
    let pt_dst = POINT {
        x: rc.left,
        y: rc.top,
    };
    let pt_src = POINT { x: 0, y: 0 };
    let size = SIZE {
        cx: vis_w as i32,
        cy: vis_h as i32,
    };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    unsafe {
        let screen_dc = GetDC(HWND(null_mut()));
        let _ = UpdateLayeredWindow(
            hwnd,
            screen_dc,
            Some(&pt_dst as *const POINT),
            Some(&size as *const SIZE),
            back_dc,
            Some(&pt_src as *const POINT),
            COLORREF(0),
            Some(&blend as *const BLENDFUNCTION),
            ULW_ALPHA,
        );
        let _ = ReleaseDC(HWND(null_mut()), screen_dc);
    }
}

fn ensure_dib(
    back_dc: HDC,
    hbmp: &mut HBITMAP,
    dib_bits: &mut *mut c_void,
    dib_size: &mut (u32, u32),
    w: u32,
    h: u32,
) {
    if *dib_size == (w, h) {
        return;
    }
    unsafe {
        if !hbmp.0.is_null() {
            let _ = DeleteObject(HGDIOBJ(hbmp.0));
        }
    }
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: w * h * 4,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [RGBQUAD::default(); 1],
    };
    let mut bits: *mut c_void = null_mut();
    let hbmp_new = unsafe {
        CreateDIBSection(
            back_dc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            HANDLE(null_mut()),
            0,
        )
        .unwrap_or_default()
    };
    unsafe {
        let _ = SelectObject(back_dc, HGDIOBJ(hbmp_new.0));
    }
    *hbmp = hbmp_new;
    *dib_bits = bits;
    *dib_size = (w, h);
}

fn toggle_expand(state: &mut AppState) {
    set_panel_open(state, !state.view.expanded());
}

/// 展开/收起胶囊面板。
/// 固定到任务栏时三环常驻任务栏、不参与形变：展开＝显示并聚焦面板窗口，收起＝直接隐藏。
/// 注意用「按下时的意图」而不是「当前状态取反」来调用：三环窗口是任务栏的从属窗口，
/// 按下它会先让面板失焦（WM_KILLFOCUS → collapse），若抬起时再按当前状态取反会把面板又展开。
fn set_panel_open(state: &mut AppState, open: bool) {
    dbg_log(&format!("set_panel_open open={open}"));
    // 展开期间挂全局鼠标钩子（点面板外任意位置收起），收起时立即撤掉；
    // 设置里关掉"点击面板外自动收起"就不挂，面板只在按 Esc / 点胶囊时收起
    if open {
        if state.config.capsule.click_outside_collapse {
            mouse_hook_install();
        }
    } else {
        mouse_hook_remove();
    }
    let changed = state.view.expanded() != open;
    if changed {
        state.view.set_expanded(open);
        let detail = if open {
            Detail::Expanded
        } else {
            Detail::Collapsed
        };
        let _ = state.cmd_tx.send(SourceCmd::SetDetail(detail));
    }
    let pinned = state.config.capsule.taskbar_pin;
    if pinned && !changed {
        return; // 固定态已处于目标状态，无需重播动画
    }
    if open {
        // 展开前先把窗口摆到位（几何变化在隐藏态完成，避免显示瞬间闪现旧位置）
        resize_window(state);
        unsafe {
            let _ = ShowWindow(state.hwnd, SW_SHOWNA);
        }
        // 注意：亚克力磨砂背景层**不再显示**。它此前只挂在固定态，导致同一透明度下
        // 三环模式的面板比横/竖排"厚"一层（面板底下多垫了一块系统磨砂）；用户已要求去掉
        // 磨砂效果，这里统一为不加背景，两种模式的透明度表现就一致了。
        // 若要恢复"两种模式都带磨砂"，改成在这里无条件 acrylic_show(state, true)
        // 并让 resize_window 里也无条件 acrylic_place。
    }
    if pinned {
        if open {
            unsafe {
                let _ = SetActiveWindow(state.hwnd);
            }
        }
        // 固定到任务栏：面板底边贴任务栏，动画从下往上长出；收起动画结束时隐藏窗口
        start_anim(state);
        return;
    }
    if !changed {
        return;
    }
    if open {
        unsafe {
            let _ = SetActiveWindow(state.hwnd);
        }
    }
    start_anim(state);
}

/// 启动/继续展开动画：16ms 定时器逐帧推进。
fn start_anim(state: &mut AppState) {
    unsafe {
        let _ = SetTimer(state.hwnd, IDT_ANIM, 16, None);
    }
    resize_window(state);
    paint_frame(state, true);
}

/// 超大卡片几何：宽 660（工作区太窄时收窄）；高恒定按 6 个卡位算，
/// 不随勾选 / 可读卡片数变化（上限为工作区高 − 24）。
/// 贴边侧跟任务栏走：任务栏竖直位于屏幕右半时改贴左侧，其余情况贴右侧。
fn big_geometry(state: &AppState) -> (u32, u32, bool) {
    let wa = monitor_work_area(unsafe { MonitorFromWindow(state.hwnd, MONITOR_DEFAULTTONEAREST) });
    let full_w = (wa.right - wa.left).max(1) as u32;
    let full_h = (wa.bottom - wa.top).max(1) as u32;
    let w = capsule_view::BIG_W.min(full_w.saturating_sub(24)).max(320);
    // 面板大小固定：不随内张小卡片增减而变（放不下的部分由工作区高度封顶）
    let h = capsule_view::big_panel_height()
        .min(full_h.saturating_sub(24))
        .max(320);
    let left = taskbar_rect().is_some_and(|tr| {
        let (tw, th) = (tr.right - tr.left, tr.bottom - tr.top);
        th > tw && tr.left > wa.left + (wa.right - wa.left) / 2
    });
    (w, h, left)
}

/// 按当前动画进度重设窗口；宽度变化时保持右缘不动（向左展开/收回）。
/// 固定在任务栏时：面板在任务栏内三环上水平居中；任务栏在下方则面板向上生长（底边贴任务栏顶）。
fn resize_window(state: &mut AppState) {
    let frame_data = match state.shared.read() {
        Ok(g) => g.grab(),
        Err(e) => {
            eprintln!("[App] RwLock poisoned in resize_window: {:?}", e);
            return;
        }
    };
    let frame = frame_data.as_frame();
    // 超大卡片：几何按工作区下发；展开期间窗口一次到位，动画只让内容滑入
    let big = if state.config.capsule.expand_big() {
        let g = big_geometry(state);
        state.view.set_big_geometry(g.0, g.1, g.2);
        Some(g)
    } else {
        None
    };
    let (w, h) = state.view.frame_size(&frame);
    // 固定态：亚克力层按“可见面板高度”跟随（底/顶边与面板一致，随内容一起生长/收回）
    let vis_h = state.view.visible_panel_h(&frame) as i32;
    let up = frame.expand_up;
    unsafe {
        let mut rc = RECT::default();
        let _ = GetWindowRect(state.hwnd, &mut rc);
        let (x, y) = if let Some((_, _, left)) = big.filter(|_| state.view.expanded()) {
            // 贴工作区侧边：上下各 12，宽度取 big_geometry 的结果
            let wa = monitor_work_area(MonitorFromWindow(state.hwnd, MONITOR_DEFAULTTONEAREST));
            let x = if left {
                wa.left + 12
            } else {
                wa.right - 12 - w as i32
            };
            (x, wa.top + 12)
        } else {
            match (state.config.capsule.taskbar_pin, state.pin_anchor) {
                (true, Some((px, py, up))) => {
                    // 面板（320 宽）在任务栏内三环（CAPSULE_W 宽）上水平居中
                    let mut x = px + (CAPSULE_W as i32 - w as i32) / 2;
                    // 居中后可能探出屏幕边缘，夹回工作区
                    let mut wa = RECT::default();
                    let _ = SystemParametersInfoW(
                        SPI_GETWORKAREA,
                        0,
                        Some(&mut wa as *mut RECT as *mut c_void),
                        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                    );
                    if wa.right > wa.left {
                        x = x.clamp(wa.left, (wa.right - w as i32).max(wa.left));
                    }
                    let y = if up {
                        taskbar_rect()
                            .map(|t| t.top)
                            .unwrap_or(py + CAPSULE_H as i32)
                            - h as i32
                    } else {
                        taskbar_rect().map(|t| t.bottom).unwrap_or(py)
                    };
                    (x, y)
                }
                _ => {
                    // 未固定：横排与"竖条贴右缘"都保持右缘不动（面板向左推/向下长）；
                    // 只有"竖条贴左缘"时保持左缘不动（面板向右推）。
                    let (mut x, mut y) = if frame.vertical && !frame.panel_left {
                        (rc.left, rc.top)
                    } else {
                        (rc.right - w as i32, rc.top)
                    };
                    // 竖排面板很长（含全部卡片可达 800+）：底边超出工作区时整体上移，
                    // 保证竖条与面板都在屏内（方案 A：面板顶边对齐竖条顶边，超了就整体上夹）。
                    // 横向同理：面板 + 竖条共 376 宽，贴边展开时可能越过本显示器边界，
                    // 不夹回来就会"露到隔壁副屏上"。
                    let wa =
                        monitor_work_area(MonitorFromWindow(state.hwnd, MONITOR_DEFAULTTONEAREST));
                    if wa.right > wa.left {
                        x = x.clamp(wa.left, (wa.right - w as i32).max(wa.left));
                    }
                    if wa.bottom > wa.top {
                        y = y.min(wa.bottom - h as i32).max(wa.top);
                    }
                    (x, y)
                }
            }
        };
        // 几何未变时跳过 SetWindowPos：固定态展开动画只动内容，窗口尺寸一次到位，
        // 而 SWP_FRAMECHANGED 每帧都触发 WM_NCCALCSIZE，是展开动画抖动的根源。
        if !(rc.left == x
            && rc.top == y
            && (rc.right - rc.left) as u32 == w
            && (rc.bottom - rc.top) as u32 == h)
        {
            let _ = SetWindowPos(
                state.hwnd,
                HWND_TOPMOST,
                x,
                y,
                w as i32,
                h as i32,
                SWP_FRAMECHANGED | SWP_NOACTIVATE,
            );
        }

        // 亚克力磨砂背景层：与可见面板同框。固定态面板一次到位、内容由底部（向上展开时）
        // 生长，故背景层底边对齐面板底边、高度取可见高度，避免动画期间露出整块灰底。
        // 超大卡片自带不透明纯色底，不需要磨砂层。
        if state.config.capsule.taskbar_pin && !state.config.capsule.expand_big() {
            let ay = if up { y + h as i32 - vis_h } else { y };
            acrylic_place(state, x, ay, w as i32, vis_h);
        }
    }
}

/// 主显示器任务栏窗口矩形（Shell_TrayWnd）。
fn taskbar_rect() -> Option<RECT> {
    let h = unsafe { FindWindowW(static_pcwstr("Shell_TrayWnd"), PCWSTR::null()) }.ok()?;
    let mut rc = RECT::default();
    unsafe { GetWindowRect(h, &mut rc) }.ok()?;
    Some(rc)
}

/// 系统托盘区左缘（TrayNotifyWnd）。可用段的右边界。
fn taskbar_tray_left(tr: &RECT) -> i32 {
    let tb =
        unsafe { FindWindowW(static_pcwstr("Shell_TrayWnd"), PCWSTR::null()) }.unwrap_or_default();
    if let Ok(h) =
        unsafe { FindWindowExW(tb, None, static_pcwstr("TrayNotifyWnd"), PCWSTR::null()) }
    {
        if !h.0.is_null() {
            let mut r = RECT::default();
            if unsafe { GetWindowRect(h, &mut r) }.is_ok() && r.left > tr.left {
                return r.left.min(tr.right);
            }
        }
    }
    tr.right
}

/// 任务栏“任务按钮区”右缘（ReBarWindow32）。可用段从左邻开始。
fn taskbar_rebar_right(tr: &RECT) -> i32 {
    let tb =
        unsafe { FindWindowW(static_pcwstr("Shell_TrayWnd"), PCWSTR::null()) }.unwrap_or_default();
    // ReBarWindow32 是任务按钮区的父容器；Win11 上其右缘即已打开窗口按钮区右缘。
    if let Ok(h) =
        unsafe { FindWindowExW(tb, None, static_pcwstr("ReBarWindow32"), PCWSTR::null()) }
    {
        if !h.0.is_null() {
            let mut r = RECT::default();
            if unsafe { GetWindowRect(h, &mut r) }.is_ok()
                && r.right > tr.left
                && r.right < tr.right
            {
                return r.right;
            }
        }
    }
    // 兜底：取任务栏左起约 250px 作为“左侧内容区”右缘，避免盖住开始/搜索/天气。
    (tr.left + 250).min(tr.right)
}

/// 收起态胶囊宽度 + 两侧最小间隙。
const PIN_NEED: i32 = CAPSULE_W as i32 + 8;
const PIN_GAP: i32 = 4;

/// 列“有内容”的判定阈值：一条竖列在任务栏高度内的 RGB 极差之和。
/// 空白列颜色均匀（≈0~20），图标/文字/小组件列色差很大。
const CONTENT_DEV: i32 = 30;
/// 托盘左侧留白：托盘溢出按钮（^）等不在 TrayNotifyWnd 内，需额外让位。
const TRAY_MARGIN: i32 = 48;

/// 抓一次任务栏条带位图，逐列判定“是否为空”（列内 RGB 极差之和 ≤ CONTENT_DEV）。
/// Win11 任务栏右侧的按钮/小组件由 XAML 合成、拿不到子窗口、UIA 也枚举不出元素，
/// 因此直接看像素。返回 [任务栏左缘, 托盘左缘 - TRAY_MARGIN) 范围内每列的空闲标志；
/// 不排除自身窗口——落位时把自身覆盖列视为空，判断遮挡时只看窗口之外的列。
fn taskbar_free_columns(tr: &RECT) -> Option<Vec<bool>> {
    let tw = tr.right - tr.left;
    let th = tr.bottom - tr.top;
    if tw <= 8 || th <= 8 {
        return None;
    }
    // 只在托盘左侧搜索，并留出托盘溢出按钮（^）的余量
    let w = (taskbar_tray_left(tr) - tr.left - TRAY_MARGIN).clamp(0, tw) as usize;
    if w == 0 {
        return None;
    }
    let mem = unsafe { CreateCompatibleDC(HDC(null_mut())) };
    if mem.0.is_null() {
        return None;
    }
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: tw,
            biHeight: -th,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: (tw * th * 4) as u32,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [RGBQUAD::default(); 1],
    };
    let mut bits: *mut c_void = null_mut();
    let bmp =
        unsafe { CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, HANDLE(null_mut()), 0) }
            .unwrap_or_default();
    let cols = if bmp.0.is_null() || bits.is_null() {
        None
    } else {
        unsafe {
            let old = SelectObject(mem, HGDIOBJ(bmp.0));
            let screen = GetDC(HWND(null_mut()));
            let ok = BitBlt(mem, 0, 0, tw, th, screen, tr.left, tr.top, SRCCOPY).is_ok();
            let _ = ReleaseDC(HWND(null_mut()), screen);
            let out = if ok {
                Some(scan_columns(bits, tw, th, w))
            } else {
                None
            };
            let _ = SelectObject(mem, old);
            out
        }
    };
    unsafe {
        if !bmp.0.is_null() {
            let _ = DeleteObject(HGDIOBJ(bmp.0));
        }
        let _ = DeleteDC(mem);
    }
    cols
}

/// 逐列判定“是否为空”：列内在任务栏高度方向上的 RGB 极差之和 ≤ CONTENT_DEV 视为空列。
fn scan_columns(bits: *const c_void, tw: i32, th: i32, w: usize) -> Vec<bool> {
    let stride = tw as usize * 4;
    let data = unsafe { std::slice::from_raw_parts(bits as *const u8, stride * th as usize) };
    let mut free = vec![false; w];
    let y0 = 4usize;
    let y1 = (th as usize).saturating_sub(4);
    for (x, f) in free.iter_mut().enumerate() {
        let mut lo = [255i32; 3];
        let mut hi = [0i32; 3];
        for y in y0..y1 {
            let off = y * stride + x * 4;
            for (c, (l, h)) in lo.iter_mut().zip(hi.iter_mut()).enumerate() {
                let v = data[off + c] as i32;
                if v < *l {
                    *l = v;
                }
                if v > *h {
                    *h = v;
                }
            }
        }
        *f = (hi[0] - lo[0]) + (hi[1] - lo[1]) + (hi[2] - lo[2]) <= CONTENT_DEV;
    }
    free
}

/// 最长连续空闲列区间 [s, e)。
fn longest_free_run(free: &[bool]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < free.len() {
        if free[i] {
            let s = i;
            while i < free.len() && free[i] {
                i += 1;
            }
            if best.is_none_or(|(bs, be)| i - s > be - bs) {
                best = Some((s, i));
            }
        } else {
            i += 1;
        }
    }
    best
}

/// 给定左缘 x 处（宽 CAPSULE_W）是否被任务栏内容挤住：只看该区间左右两侧紧邻的窄带
/// （三环窗口自身像素看不到其下方内容，故不采自身范围）；两侧都有内容才算被挤住。
fn crowded_at(free: &[bool], cur: i32, tr_left: i32) -> bool {
    const BAND: i32 = 16;
    let n = free.len() as i32;
    let probe = |a: i32, b: i32| -> Option<bool> {
        if b - a < 8 {
            return None; // 窄带太窄，判不了
        }
        Some(free[a as usize..b as usize].iter().any(|f| !*f))
    };
    let l = (cur - tr_left).clamp(0, n);
    let r = (cur - tr_left + CAPSULE_W as i32).clamp(0, n);
    matches!(
        (probe((l - BAND).max(0), l), probe(r, (r + BAND).min(n))),
        (Some(true), Some(true))
    )
}

/// 在空闲列中找一个与 base 位置不重叠、宽度足够的空位左缘（屏幕坐标）。
fn avoid_candidate(free: &[bool], base: i32, tr_left: i32) -> Option<i32> {
    let (s, e) = longest_free_run(free)?;
    let (seg0, seg1) = (tr_left + s as i32, tr_left + e as i32);
    if seg1 - seg0 < PIN_NEED {
        return None;
    }
    // 与用户位置重叠就不算候选，否则会原地打转
    if seg0 < base + CAPSULE_W as i32 && base < seg1 {
        return None;
    }
    Some(seg0 + PIN_GAP)
}

/// 三环窗口当前左缘（屏幕坐标）。用于定位滞回：任务栏透明时像素判定会随壁纸噪声抖动，
/// 若每次都吸附到空段左缘，三环会来回跑；以当前位置为首选即可保持稳定。
fn rings_current_x(state: &AppState) -> Option<i32> {
    if state.pin_hidden {
        return None;
    }
    let r = state.rings.as_ref()?;
    let mut rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(r.hwnd, &mut rc);
    }
    (rc.right > rc.left).then_some(rc.left)
}

/// 位置微调死区：与当前位置差异小于该值就不动，避免像素噪声造成的持续微移。
const PIN_DEADZONE: i32 = 8;

/// 任务栏上可放置胶囊的连续空段 [x0, x1)，结果缓存 `SEG_CACHE`。
/// 用像素扫描找视觉空段；失败时回退到托盘左侧的空档。
/// 扫描要抓整条任务栏位图，开销不小，不宜每 500ms 都做一遍。
const SEG_CACHE: std::time::Duration = std::time::Duration::from_secs(10);

fn taskbar_free_segment(state: &mut AppState) -> Option<(i32, i32)> {
    let tr = taskbar_rect();
    let key = tr.map(|t| (t.left, t.top, t.right, t.bottom));
    // 缓存有效条件：未过期且任务栏矩形未变（分辨率/任务栏方向变化时立即重算）
    if state.seg_at.is_some_and(|t| t.elapsed() < SEG_CACHE) && state.seg_rect == key {
        return state.seg;
    }
    let seg = match tr {
        Some(tr) => taskbar_free_segment_uncached(state, &tr),
        None => None,
    };
    state.seg = seg;
    state.seg_rect = key;
    state.seg_at = Some(Instant::now());
    seg
}

fn taskbar_free_segment_uncached(state: &mut AppState, tr: &RECT) -> Option<(i32, i32)> {
    if let Some(mut free) = taskbar_free_columns(tr) {
        // 首次落位：把三环自身覆盖的列视为空（它本来就该待在空段里）
        if let Some(r) = state.rings.as_ref() {
            let mut rc = RECT::default();
            unsafe {
                let _ = GetWindowRect(r.hwnd, &mut rc);
            }
            let n = free.len() as i32;
            let l = (rc.left - tr.left).clamp(0, n) as usize;
            let rr = (rc.right - tr.left).clamp(0, n) as usize;
            if l < rr {
                free[l..rr].fill(true);
            }
        }
        if let Some((s, e)) = longest_free_run(&free) {
            let seg = (tr.left + s as i32, tr.left + e as i32);
            if seg.1 - seg.0 >= PIN_NEED {
                return Some(seg);
            }
        }
    }
    // 兜底：紧靠托盘左侧（任务按钮与托盘之间通常为空）
    let right = taskbar_tray_left(tr);
    let left = (right - PIN_GAP - CAPSULE_W as i32).max(taskbar_rebar_right(tr) + PIN_GAP);
    (right - left >= CAPSULE_W as i32).then_some((left, right))
}

/// 三环可放置的横向范围 [x0, x1)：整条任务栏（任务栏左缘 → 托盘左缘）。
/// 位置交给用户拖动决定，不再被像素扫描出的“空段”限制；扫描只用于首次落位。
fn taskbar_span(tr: &RECT) -> Option<(i32, i32)> {
    let x0 = tr.left + PIN_GAP;
    let x1 = taskbar_tray_left(tr) - PIN_GAP - TRAY_MARGIN_EDGE;
    (x1 - x0 >= CAPSULE_W as i32).then_some((x0, x1))
}

/// 任务栏右端托盘区前的让位（托盘溢出按钮 ^ 等不在 TrayNotifyWnd 内）。
const TRAY_MARGIN_EDGE: i32 = 8;

/// 在可用段 [x0, x1) 内为宽度 CAPSULE_W 的胶囊选一个合法左缘。
/// `prefer_x` 为用户拖动选定的左缘（None 表示取最左可用位置）。
fn pin_fit(x0: i32, x1: i32, prefer_x: Option<i32>) -> i32 {
    let w = CAPSULE_W as i32;
    let max_x = (x1 - w - PIN_GAP).max(x0 + PIN_GAP);
    if let Some(px) = prefer_x {
        return px.clamp(x0 + PIN_GAP, max_x);
    }
    x0 + PIN_GAP
}

/// 采样任务栏左端空白处颜色，判断任务栏是否为浅色（决定透明融合时的文字配色）。
fn taskbar_is_light(tr: &RECT) -> bool {
    let dc = unsafe { GetDC(HWND(null_mut())) };
    if dc.0.is_null() {
        return true;
    }
    let c = unsafe { GetPixel(dc, tr.left + 4, (tr.top + tr.bottom) / 2) };
    unsafe {
        let _ = ReleaseDC(HWND(null_mut()), dc);
    }
    if c.0 == u32::MAX {
        return true; // CLR_INVALID：取不到按浅色处理
    }
    let r = (c.0 & 0xFF) as u32;
    let g = ((c.0 >> 8) & 0xFF) as u32;
    let b = ((c.0 >> 16) & 0xFF) as u32;
    (r * 299 + g * 587 + b * 114) / 1000 > 128
}

/// 同步“固定在任务栏”的界面状态：任务栏内三环窗口的创建/定位/配色；无空段时隐藏三环。
fn sync_taskbar_state(state: &mut AppState) {
    // 拖动三环的过程中由拖动逻辑接管定位，避免周期重排把它拽回
    if state.rings.as_ref().is_some_and(|r| r.dragging) {
        return;
    }
    // 展开态（面板浮在任务栏上方）：三环保持原位，跳过重算
    if state.view.expanded() {
        return;
    }
    let pinned = state.config.capsule.taskbar_pin;
    let mut tb_h = 0i32;
    let mut light = true;
    let mut up = false;
    let mut anchor: Option<(i32, i32, bool)> = None;
    let mut fits = false;
    if pinned {
        if let Some(tr) = taskbar_rect() {
            tb_h = tr.bottom - tr.top;
            light = taskbar_is_light(&tr);
            let horizontal = (tr.right - tr.left) >= tb_h;
            let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
            if horizontal {
                // 位置策略：整条任务栏都是可用范围（拖动可放任意处）；
                // 优先级 = 用户拖动保存的位置 > 当前位置 > 首次用像素扫描挑一个空位。
                // 一旦有了位置就不再重算，避免透明任务栏下"任务栏后面内容变化"把三环带跑。
                if let Some((sx0, sx1)) = taskbar_span(&tr) {
                    fits = true;
                    let cur = rings_current_x(state);
                    // 用户位置：拖动保存的位置优先，没有就用当前位置
                    let base = state.config.capsule.taskbar_x.or(cur);
                    // 遮挡兜底：只在“用户位置被任务栏内容挤住”时才搬，且连续两次判定一致才生效。
                    // 透明任务栏下像素判定会被壁纸带偏，连续确认 + 必须存在不重叠的空位，可挡掉误判。
                    if let Some(bx) = base {
                        if state.pin_check_at.is_none_or(|t| t.elapsed() >= SEG_CACHE) {
                            state.pin_check_at = Some(Instant::now());
                            if let Some(free) = taskbar_free_columns(&tr) {
                                if crowded_at(&free, bx, tr.left) {
                                    if state.pin_avoid_x.is_none() {
                                        match avoid_candidate(&free, bx, tr.left) {
                                            Some(cx) => {
                                                let same = state.pin_avoid_cand.is_some_and(|p| {
                                                    (p - cx).abs() <= PIN_DEADZONE
                                                });
                                                state.pin_avoid_hits = if same {
                                                    state.pin_avoid_hits.saturating_add(1)
                                                } else {
                                                    1
                                                };
                                                state.pin_avoid_cand = Some(cx);
                                                if state.pin_avoid_hits >= 2 {
                                                    state.pin_avoid_x = Some(cx);
                                                    state.pin_avoid_hits = 0;
                                                }
                                            }
                                            None => {
                                                state.pin_avoid_cand = None;
                                                state.pin_avoid_hits = 0;
                                            }
                                        }
                                    }
                                } else {
                                    // 不再拥挤：撤销兜底，回到用户位置
                                    state.pin_avoid_cand = None;
                                    state.pin_avoid_hits = 0;
                                    state.pin_avoid_x = None;
                                }
                            }
                        }
                    }
                    let prefer = match state.pin_avoid_x.or(base) {
                        Some(px) => Some(px),
                        // 首次落位：取扫描空段的左缘（扫描结果不可信时会走兜底）
                        None => taskbar_free_segment(state).map(|(fx0, _)| fx0 + PIN_GAP),
                    };
                    let mut ax = pin_fit(sx0, sx1, prefer);
                    if state.pin_avoid_x.is_none() {
                        if let Some(cx) = cur {
                            if (ax - cx).abs() < PIN_DEADZONE {
                                ax = cx; // 差异极小：保持原位，不做微调
                            }
                        }
                    }
                    // 三环高与任务栏等高，纵向铺满任务栏条
                    let y = tr.top + (tb_h - capsule_view::RINGS_H as i32) / 2;
                    // 任务栏在屏幕下半部时面板向上展开，避免展开后跑出屏幕
                    up = (y + CAPSULE_H as i32 / 2) > screen_h / 2;
                    anchor = Some((ax, y, up));
                }
            } else {
                // 竖直任务栏：贴最上端、水平居中
                let cw = CAPSULE_W as i32;
                anchor = Some((tr.left + ((tr.right - tr.left) - cw) / 2, tr.top + 6, false));
                fits = true;
            }
        }
    }
    {
        let mut sh = match state.shared.write() {
            Ok(g) => g,
            Err(e) => {
                eprintln!("[App] RwLock poisoned in sync_taskbar_state: {:?}", e);
                return;
            }
        };
        if sh.pinned != pinned || sh.taskbar_light != light || sh.expand_up != (pinned && up) {
            sh.pinned = pinned;
            sh.taskbar_light = light;
            sh.expand_up = pinned && up;
            sh.bump_seq();
        }
    }
    let mut dirty = false;
    if let Some(a) = anchor {
        if state.pin_anchor != Some(a) {
            state.pin_anchor = Some(a);
            dirty = true;
        }
    }
    if state.view.set_taskbar_style(tb_h, false) {
        dirty = true;
    }
    // 固定但任务栏放不下 → 整体隐藏（托盘图标仍可查看/操作）
    if pinned {
        // 三环常驻任务栏内（Shell_TrayWnd 子窗口）；主窗口收起态隐藏，只作展开面板用
        if let Some((ax, ay, _)) = anchor {
            if rings_ensure(state) {
                rings_place(state, ax, ay);
                rings_paint(state, false);
            }
        }
        if !fits {
            if let Some(r) = state.rings.as_ref() {
                unsafe {
                    let _ = ShowWindow(r.hwnd, SW_HIDE);
                }
            }
        }
        state.pin_hidden = !fits;
        unsafe {
            let _ = ShowWindow(state.hwnd, SW_HIDE);
        }
        // 收起态不显示磨砂背景层（否则桌面残留无内容灰底）
        acrylic_show(state, false);
    } else {
        rings_destroy(state);
        state.pin_hidden = false;
        unsafe {
            let _ = ShowWindow(state.hwnd, SW_SHOWNA);
        }
        if dirty {
            resize_window(state);
            paint_frame(state, true);
        }
    }
}

/// 全屏应用检测：前台窗口若占满整块显示器、且与本窗口在同一块屏上，返回 true。
///
/// 边界用 DWM 的「扩展边框」而不是 GetWindowRect——游戏 / UWP 窗口的不可见边框
/// 和阴影会让普通矩形偏大，扩展边框才是肉眼看到的实际边界。
/// 桌面 / 任务栏铺满整屏但不是"应用"，按窗口类名排除；本程序自己的窗口也排除。
fn fullscreen_on_same_monitor(state: &AppState) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() || fg == state.hwnd {
            return false;
        }
        if !state.settings_hwnd.0.is_null() && fg == state.settings_hwnd {
            return false;
        }
        let mut buf = [0u16; 64];
        let n = GetClassNameW(fg, &mut buf);
        if n > 0 {
            let name = String::from_utf16_lossy(&buf[..n as usize]);
            if matches!(
                name.as_str(),
                "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
            ) {
                return false;
            }
        }
        let mut rc = RECT::default();
        let ok = DwmGetWindowAttribute(
            fg,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rc as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok();
        if !ok {
            let _ = GetWindowRect(fg, &mut rc);
        }
        if rc.right <= rc.left || rc.bottom <= rc.top {
            return false;
        }
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        if mon.0.is_null() {
            return false;
        }
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return false;
        }
        // 允许 2px 误差：个别窗口的边缘会差一两个像素
        const TOL: i32 = 2;
        let m = mi.rcMonitor;
        let covers = rc.left <= m.left + TOL
            && rc.top <= m.top + TOL
            && rc.right >= m.right - TOL
            && rc.bottom >= m.bottom - TOL;
        if !covers {
            return false;
        }
        // 只在"全屏应用与本窗口在同一块屏"时隐藏：副屏打游戏不该把主屏的胶囊也藏掉
        let own = MonitorFromWindow(own_visible_hwnd(state), MONITOR_DEFAULTTONEAREST);
        own.0 == mon.0
    }
}

/// 当前代表悬浮窗的句柄：固定在任务栏时取三环窗口（此时主窗口常处于隐藏态）。
fn own_visible_hwnd(state: &AppState) -> HWND {
    if state.config.capsule.taskbar_pin {
        if let Some(r) = state.rings.as_ref() {
            return r.hwnd;
        }
    }
    state.hwnd
}

/// 每 500ms 评估一次：全屏应用运行时隐藏悬浮窗，退出全屏后按原形态恢复。
fn sync_fullscreen_state(state: &mut AppState) {
    if !state.config.capsule.hide_on_fullscreen {
        restore_after_fullscreen(state);
        return;
    }
    // 设置窗开着时不做判定：用户正在调设置，别把窗口藏了
    if !state.settings_hwnd.0.is_null() {
        return;
    }
    let fullscreen = fullscreen_on_same_monitor(state);
    if fullscreen == state.fs_hidden {
        return;
    }
    if fullscreen {
        hide_for_fullscreen(state);
    } else {
        restore_after_fullscreen(state);
    }
}

/// 隐藏悬浮窗（主窗口 + 任务栏三环 + 磨砂层）。
fn hide_for_fullscreen(state: &mut AppState) {
    state.fs_hidden = true;
    mouse_hook_remove(); // 面板已被藏起来，钩子留着只会误收起
    unsafe {
        let _ = ShowWindow(state.hwnd, SW_HIDE);
        if let Some(r) = state.rings.as_ref() {
            let _ = ShowWindow(r.hwnd, SW_HIDE);
        }
    }
    acrylic_show(state, false);
}

/// 退出全屏后恢复：固定态恢复三环（展开中则连面板一起），普通态恢复胶囊 / 面板。
fn restore_after_fullscreen(state: &mut AppState) {
    if !state.fs_hidden {
        return;
    }
    state.fs_hidden = false;
    if state.pin_hidden {
        return; // 任务栏没有空段，本来就该保持隐藏
    }
    if state.config.capsule.taskbar_pin {
        if let Some((ax, ay, _)) = state.pin_anchor {
            rings_place(state, ax, ay); // 带 SWP_SHOWWINDOW
            rings_paint(state, true);
        }
        if state.view.expanded() {
            resize_window(state);
            unsafe {
                let _ = ShowWindow(state.hwnd, SW_SHOWNA);
            }
            paint_frame(state, true);
            if state.config.capsule.click_outside_collapse {
                mouse_hook_install();
            }
        } else {
            sync_taskbar_state(state);
        }
    } else {
        resize_window(state);
        unsafe {
            let _ = ShowWindow(state.hwnd, SW_SHOWNA);
        }
        paint_frame(state, true);
        if state.view.expanded() && state.config.capsule.click_outside_collapse {
            mouse_hook_install();
        }
    }
}

/// 应用/取消“固定在任务栏”：三环挂成任务栏的从属窗口常驻其上方，展开面板为独立浮窗。
fn apply_taskbar_pin(state: &mut AppState) {
    if !state.config.capsule.taskbar_pin {
        state.pin_anchor = None;
        state.view.set_hovered(false);
        unsafe {
            let _ = KillTimer(state.hwnd, IDT_PIN);
        }
        // 取消固定：销毁任务栏内的三环子窗口，收起面板，恢复普通胶囊
        state.view.set_expanded(false);
        state.view.jump_anim();
        sync_taskbar_state(state); // 恢复普通布局与配色，并显示主窗口
        resize_window(state); // 先收回到收起态尺寸
                              // 再回到该方向的默认落点：固定期间主窗口是隐藏的，矩形还停在面板/三环的残留几何上，
                              // 直接沿用会压在屏幕右上角——正好是最大化窗口的最小化/关闭按钮那一块。
        let vertical = state.config.capsule.orientation_hint() == Some(true);
        let (cw, _) = collapsed_main_size(vertical);
        let (dx, dy) = default_capsule_pos(cw as i32, vertical);
        {
            let mut sh = match state.shared.write() {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("[App] RwLock poisoned in apply_taskbar_pin: {:?}", e);
                    return;
                }
            };
            if sh.vertical != vertical || sh.panel_left {
                sh.vertical = vertical;
                sh.panel_left = false;
                sh.bump_seq();
            }
        }
        state.config.capsule.x = Some(dx);
        state.config.capsule.y = Some(dy);
        let _ = crate::config::save(&state.config);
        unsafe {
            let _ = SetWindowPos(
                state.hwnd,
                None,
                dx,
                dy,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        resize_window(state);
        paint_frame(state, true);
        return;
    }
    if taskbar_rect().is_none() {
        state.pin_anchor = None;
        return;
    }
    // 固定到任务栏：先收起面板（不做形变），再由三环子窗口接管收起态显示
    state.view.set_expanded(false);
    state.view.jump_anim();
    sync_taskbar_state(state); // 计算槽位、创建并定位任务栏内的三环子窗口
    unsafe {
        let _ = SetTimer(state.hwnd, IDT_PIN, 500, None);
    }
}

fn show_tray_menu(state: &mut AppState) {
    let menu = unsafe { CreatePopupMenu().unwrap() };
    unsafe {
        // 「胶囊 / 三环 切换」：勾选态表示当前是"挂在任务栏的三环"形态。
        // 原先这个开关放设置窗里叫"固定到任务栏"，放托盘右键更顺手（不用开设置窗）。
        let pin_flags = MF_STRING
            | if state.config.capsule.taskbar_pin {
                MF_CHECKED
            } else {
                MF_UNCHECKED
            };
        let _ = AppendMenuW(
            menu,
            pin_flags,
            ID_TRAY_PIN as usize,
            pcwstr("胶囊/三环切换"),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_TRAY_SETTINGS as usize, pcwstr("设置…"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_TRAY_EXIT as usize, pcwstr("退出"));
    }
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(state.hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            0,
            state.hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
    }
}

// ---- 拖动吸附：靠近工作区边缘自动贴齐 ----

const SNAP_PX: i32 = 12;

fn snap_to_work_area(hwnd: HWND, w: i32, h: i32, nx: &mut i32, ny: &mut i32) {
    let hmon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if hmon.is_invalid() {
        return;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(hmon, &mut info) }.as_bool() {
        let wa = info.rcWork;
        if (*nx - wa.left).abs() < SNAP_PX {
            *nx = wa.left;
        }
        if ((*nx + w) - wa.right).abs() < SNAP_PX {
            *nx = wa.right - w;
        }
        if (*ny - wa.top).abs() < SNAP_PX {
            *ny = wa.top;
        }
        if ((*ny + h) - wa.bottom).abs() < SNAP_PX {
            *ny = wa.bottom - h;
        }
    }
}

// ---- 设置窗口：NAS 地址输入 ----

const SETTINGS_CLASS: &str = "HeTaoCapsuleSettings";
const IDC_ADDR_EDIT: usize = 101;
const IDC_TOKEN_EDIT: usize = 102;
const IDC_TEST_BUTTON: usize = 103;
const IDC_TEST_RESULT: usize = 104;
/// 胶囊 / 三环 切换（与托盘右键菜单里那项是同一个开关）
const IDC_TASKBAR_CHK: usize = 105;
/// 点击面板外自动收起（默认开启）
const IDC_CLICK_OUTSIDE_CHK: usize = 106;
/// 主题 2 档单选（顺序同 config::THEMES：亮色 / 暗色）
const IDC_THEME_BASE: usize = 150;
const THEME_LABELS: [&str; 2] = ["亮色", "暗色"];
/// 三个分组小标题（只有它们用系统灰色绘制）
const IDC_SEC_CONN: usize = 107;
const IDC_SEC_CARDS: usize = 108;
const IDC_SEC_OTHER: usize = 109;
/// 卡片选择提示行（全部取消时提示"请至少保留一张卡片"）
const IDC_CARDS_HINT: usize = 110;
/// 全屏应用时自动隐藏悬浮窗（默认开启）
const IDC_FS_CHK: usize = 111;
/// 六个卡片复选（顺序同 CardVis::all：CPU/内存/显卡/网络/硬盘读写/存储状态）
const IDC_CARD_BASE: usize = 120;
const CARD_COUNT: usize = 6;
const CARD_LABELS: [&str; CARD_COUNT] = ["CPU", "内存", "显卡", "网络", "硬盘读写", "存储状态"];
/// 透明度 4 档单选（顺序同 OPACITY_STEPS）
const IDC_OPACITY_BASE: usize = 130;
/// 显示方向 3 档单选（顺序同 config::ORIENTATIONS：自动 / 横向 / 竖向）
const IDC_ORIENT_BASE: usize = 140;
/// 展开样式单选（小面板 / 超大卡片）
const IDC_STYLE_BASE: usize = 160;
const ORIENT_LABELS: [&str; 3] = ["自动", "横向", "竖向"];
/// 展开样式档位（顺序同 config::EXPAND_STYLES）
const STYLE_LABELS: [&str; 2] = ["小面板", "超大卡片"];
/// 设置窗口客户区尺寸（三段式：连接 / 显示卡片 / 其他）
const SETTINGS_CLIENT_W: i32 = 420;
const SETTINGS_CLIENT_H: i32 = 578;
/// 按钮通知码 BN_CLICKED = 0（不依赖 crate 常量类型）
const BN_CLICKED_CODE: u32 = 0;
/// 静态控件样式 SS_ETCHEDHORZ = 0x10：一条 1px 蚀刻横线（分组分隔）
const SS_ETCHEDHORZ_STYLE: u32 = 0x0010;

/// 设置窗定位锚点：决定窗口从哪里长出来（两种入口都尽量让用户少移动鼠标）。
#[derive(Clone, Copy)]
enum SettingsAnchor {
    /// 贴展开面板左侧、纵向对齐齿轮（面板里点齿轮打开）
    Panel,
    /// 在光标处弹出（从托盘菜单打开，鼠标就在托盘上）
    Cursor,
}

/// 指定显示器的工作区（已排除任务栏）；拿不到时回退主屏工作区。
fn monitor_work_area(mon: HMONITOR) -> RECT {
    unsafe {
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            return mi.rcWork;
        }
        let mut wa = RECT::default();
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut wa as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        wa
    }
}

fn open_settings(state: &mut AppState, anchor: SettingsAnchor) {
    if !state.settings_hwnd.0.is_null() {
        unsafe {
            let _ = SetForegroundWindow(state.settings_hwnd);
        }
        return;
    }
    let instance = unsafe { GetModuleHandleW(None).unwrap_or_default() };

    // 注册设置窗口类（只注册一次）
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(settings_wndproc),
            hInstance: instance.into(),
            lpszClassName: {
                let v: Vec<u16> = SETTINGS_CLASS.encode_utf16().chain(Some(0)).collect();
                PCWSTR(Box::leak(v.into_boxed_slice()).as_ptr())
            },
            hIcon: app_icons().0,
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut c_void),
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }
    });

    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE;

    // 子控件字体：系统 UI 字体（Win11 上 Segoe UI，中文回退微软雅黑），
    // 比 DEFAULT_GUI_FONT 的老式点阵更贴近系统原生观感。
    state.settings_font = create_ui_font();

    // 按客户区尺寸反推窗口外框（客户区尺寸固定，标题栏/边框由系统加上）
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: SETTINGS_CLIENT_W,
        bottom: SETTINGS_CLIENT_H,
    };
    unsafe {
        let _ = AdjustWindowRect(&mut frame, style, BOOL(0));
    }
    let (w, h) = (frame.right - frame.left, frame.bottom - frame.top);

    // 定位：托盘来的在光标处弹出（底边贴光标，像弹出菜单那样往上长）；
    // 面板齿轮来的贴面板左侧、纵向对齐齿轮——两种入口都尽量让用户少移动鼠标。
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    let (x, y) = match anchor {
        SettingsAnchor::Cursor => {
            let wa = monitor_work_area(unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) });
            let max_x = (wa.right - w).max(wa.left);
            let max_y = (wa.bottom - h).max(wa.top);
            (pt.x.clamp(wa.left, max_x), (pt.y - h).clamp(wa.top, max_y))
        }
        SettingsAnchor::Panel => {
            let wa = monitor_work_area(unsafe {
                MonitorFromWindow(state.hwnd, MONITOR_DEFAULTTONEAREST)
            });
            let mut rc = RECT::default();
            unsafe {
                let _ = GetWindowRect(state.hwnd, &mut rc);
            }
            let gap = 12;
            let max_x = (wa.right - w).max(wa.left);
            let max_y = (wa.bottom - h).max(wa.top);
            let mut x = rc.left - w - gap;
            if x < wa.left {
                x = rc.right + gap;
            }
            let x = x.clamp(wa.left, max_x);
            // 竖排展开：面板本身就从窗口顶部开始长，此时按"纵向居中于齿轮"会把设置窗
            // 夹到屏幕最上沿、比面板还高一截；改成顶边对齐面板顶边，两块从同一条水平线开始。
            let vertical_panel = {
                let sh = match state.shared.read() {
                    Ok(g) => g,
                    Err(e) => {
                        eprintln!("[App] RwLock poisoned in show_settings vertical_panel: {:?}", e);
                        return;
                    }
                };
                sh.vertical && state.view.expanded()
            };
            let y = if vertical_panel {
                rc.top
            } else {
                // 齿轮一行的纵向中心（与标题行绘制位置对应）
                let expand_up = match state.shared.read() {
                    Ok(g) => g.expand_up,
                    Err(e) => {
                        eprintln!("[App] RwLock poisoned in show_settings expand_up: {:?}", e);
                        return;
                    }
                };
                let pinned = state.config.capsule.taskbar_pin;
                let gear_cy = if !pinned {
                    rc.top + CAPSULE_H as i32 / 2
                } else if expand_up {
                    rc.bottom - capsule_view::HEADER_ROW_CY
                } else {
                    rc.top + capsule_view::HEADER_ROW_CY
                };
                gear_cy - h / 2
            };
            (x, y.clamp(wa.top, max_y))
        }
    };

    let class_wide: Vec<u16> = SETTINGS_CLASS.encode_utf16().chain(Some(0)).collect();
    // 即时预览的"还原点"：点取消 / 直接关窗时，用它把主题与透明度退回打开前的样子
    state.settings_orig = Some((
        state.config.capsule.theme.clone(),
        state.config.capsule.opacity,
    ));
    // 勾选顺序的起点：面板里当前的卡片顺序，之后每勾一张就挪到末尾
    state.settings_card_order = state.config.capsule.cards.order;
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_TOPMOST,
            PCWSTR(class_wide.as_ptr()),
            pcwstr("核桃胶囊 · 设置"),
            style,
            x,
            y,
            w,
            h,
            state.hwnd,
            None,
            instance,
            Some(state as *mut AppState as *const c_void),
        )
        .unwrap_or_default()
    };
    if hwnd.is_invalid() {
        // 创建失败：立刻释放刚建好的字体，避免句柄泄漏
        if !state.settings_font.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(state.settings_font.0));
            }
            state.settings_font = HFONT(null_mut());
        }
    }
    state.settings_hwnd = hwnd;
}

/// 系统 UI 字体：SPI_GETNONCLIENTMETRICS 的 lfMessageFont。失败返回空句柄
/// （子控件退回系统默认字体，不影响可用性）。
fn create_ui_font() -> HFONT {
    unsafe {
        let mut ncm: NONCLIENTMETRICSW = std::mem::zeroed();
        ncm.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        let ok = SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            ncm.cbSize,
            Some(&mut ncm as *mut NONCLIENTMETRICSW as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok();
        if ok {
            return CreateFontIndirectW(&ncm.lfMessageFont);
        }
        HFONT(null_mut())
    }
}

/// 给子控件套用设置窗字体（没有自建字体时退回系统 GUI 字体）。
fn child_font(hwnd: HWND, font: HFONT) {
    let f = if font.is_invalid() {
        unsafe { GetStockObject(DEFAULT_GUI_FONT) }
    } else {
        HGDIOBJ(font.0)
    };
    unsafe {
        let _ = SendMessageW(hwnd, WM_SETFONT, WPARAM(f.0 as usize), LPARAM(1));
    }
}

/// 读取复选/单选钮的勾选态。
fn is_checked(hwnd: HWND, id: usize) -> bool {
    unsafe {
        let ctl = GetDlgItem(hwnd, id as i32).unwrap_or_default();
        SendMessageW(ctl, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 != 0
    }
}

/// 读取设置窗里六个卡片开关。
fn read_cards(hwnd: HWND) -> CardVis {
    let v: [bool; CARD_COUNT] = std::array::from_fn(|i| is_checked(hwnd, IDC_CARD_BASE + i));
    let mut cv = CardVis::default();
    cv.set_all(v);
    cv
}

/// 读取透明度单选（返回最接近的档位值）。
fn read_opacity(hwnd: HWND) -> f32 {
    (0..OPACITY_STEPS.len())
        .find(|&i| is_checked(hwnd, IDC_OPACITY_BASE + i))
        .map_or(OPACITY_STEPS[0], |i| OPACITY_STEPS[i])
}

/// 设置窗即时预览：把窗里当前选中的主题/透明度写进**内存**配置并立刻重绘，不写盘。
/// 主题是"按主题构建面板"的，所以还要 bump_seq 让后台线程按新主题重建一份。
fn apply_settings_preview(state: &mut AppState, hwnd: HWND) {
    let theme_idx = (0..THEME_LABELS.len())
        .find(|&i| is_checked(hwnd, IDC_THEME_BASE + i))
        .unwrap_or(0);
    state.config.capsule.theme = crate::config::THEMES[theme_idx].to_string();
    state.config.capsule.opacity = read_opacity(hwnd);
    let dark = state.config.capsule.theme_dark();
    {
        let mut sh = match state.shared.write() {
            Ok(g) => g,
            Err(e) => {
                eprintln!("[App] RwLock poisoned in apply_settings_preview: {:?}", e);
                return;
            }
        };
        sh.dark = dark;
        sh.bump_seq();
    }
    paint_frame(state, true);
    rings_paint(state, true); // 任务栏三环是独立窗口，单独刷一次
}

/// 还原即时预览（点"取消"或直接关窗时）：退回打开设置窗时的主题与透明度并重绘。
/// 快照已被"确定"取走（None）时什么都不做。
fn restore_settings_preview(state: &mut AppState) {
    let Some((theme, opacity)) = state.settings_orig.take() else {
        return;
    };
    state.config.capsule.theme = theme;
    state.config.capsule.opacity = opacity;
    let dark = state.config.capsule.theme_dark();
    {
        let mut sh = match state.shared.write() {
            Ok(g) => g,
            Err(e) => {
                eprintln!("[App] RwLock poisoned in restore_settings_preview: {:?}", e);
                return;
            }
        };
        sh.dark = dark;
        sh.bump_seq();
    }
    paint_frame(state, true);
    rings_paint(state, true);
}

fn settings_wndproc_impl(
    state: &mut AppState,
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                const PAD: i32 = 16;
                const CW: i32 = SETTINGS_CLIENT_W - PAD * 2; // 内容宽（388）
                let font = state.settings_font;
                let mk_child = |class: &str,
                                text: &str,
                                style: WINDOW_STYLE,
                                x: i32,
                                y: i32,
                                w: i32,
                                h: i32,
                                id: usize| {
                    let h = CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        pcwstr(class),
                        pcwstr(text),
                        WS_CHILD | WS_VISIBLE | style,
                        x,
                        y,
                        w,
                        h,
                        hwnd,
                        HMENU(id as *mut c_void),
                        GetModuleHandleW(None).unwrap_or_default(),
                        None,
                    )
                    .unwrap_or_default();
                    child_font(h, font);
                    h
                };
                let set_check = |id: usize, on: bool| {
                    let ctl = GetDlgItem(hwnd, id as i32).unwrap_or_default();
                    let _ = SendMessageW(ctl, BM_SETCHECK, WPARAM(on as usize), LPARAM(0));
                };

                // 三组纵向布局：连接 / 显示卡片 / 其他，底部按钮贴右下角。
                // 控件坐标都按各组的基准 y 偏移书写，调组间距只改这四个数。
                let c0 = 16; // 连接组
                let k0 = 192; // 显示卡片组
                let o0 = 344; // 其他组
                let btn_y = 532; // 底部按钮

                // ---- 分组一：连接 ----
                mk_child(
                    "STATIC",
                    "连接",
                    WINDOW_STYLE(0),
                    PAD,
                    c0,
                    200,
                    18,
                    IDC_SEC_CONN,
                );
                mk_child(
                    "STATIC",
                    "地址（host:port）",
                    WINDOW_STYLE(0),
                    PAD,
                    c0 + 22,
                    CW,
                    16,
                    0,
                );
                mk_child(
                    "EDIT",
                    &state.config.nas.addr(),
                    WINDOW_STYLE(WS_BORDER.0 | ES_AUTOHSCROLL as u32),
                    PAD,
                    c0 + 40,
                    CW,
                    26,
                    IDC_ADDR_EDIT,
                );
                mk_child(
                    "STATIC",
                    "配对令牌（见 NAS 管理页）",
                    WINDOW_STYLE(0),
                    PAD,
                    c0 + 74,
                    CW,
                    16,
                    0,
                );
                mk_child(
                    "EDIT",
                    &state.config.nas.token,
                    WINDOW_STYLE(WS_BORDER.0 | ES_AUTOHSCROLL as u32),
                    PAD,
                    c0 + 92,
                    CW,
                    26,
                    IDC_TOKEN_EDIT,
                );
                mk_child(
                    "BUTTON",
                    "测试连接",
                    WINDOW_STYLE(0),
                    PAD,
                    c0 + 126,
                    88,
                    28,
                    IDC_TEST_BUTTON,
                );
                mk_child(
                    "STATIC",
                    "",
                    WINDOW_STYLE(0),
                    PAD + 98,
                    c0 + 132,
                    CW - 98,
                    18,
                    IDC_TEST_RESULT,
                );
                mk_child(
                    "STATIC",
                    "",
                    WINDOW_STYLE(SS_ETCHEDHORZ_STYLE),
                    PAD,
                    c0 + 164,
                    CW,
                    2,
                    0,
                );

                // ---- 分组二：显示卡片（两列三行）----
                mk_child(
                    "STATIC",
                    "显示卡片",
                    WINDOW_STYLE(0),
                    PAD,
                    k0,
                    200,
                    18,
                    IDC_SEC_CARDS,
                );
                mk_child(
                    "STATIC",
                    "勾选顺序即面板排列顺序（后勾的排到后面）；收起态不受影响",
                    WINDOW_STYLE(0),
                    PAD,
                    k0 + 18,
                    CW,
                    16,
                    0,
                );
                for (i, label) in CARD_LABELS.iter().enumerate() {
                    let col = (i % 2) as i32;
                    let row = (i / 2) as i32;
                    let chk = mk_child(
                        "BUTTON",
                        label,
                        WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                        PAD + col * 198,
                        k0 + 40 + row * 24,
                        190,
                        22,
                        IDC_CARD_BASE + i,
                    );
                    let _ = chk;
                    set_check(IDC_CARD_BASE + i, state.config.capsule.cards.all()[i]);
                }
                // 回显当前排列顺序（与点击复选后的回显一致）
                let names: Vec<&str> = state
                    .config
                    .capsule
                    .cards
                    .ordered()
                    .iter()
                    .map(|&i| CARD_LABELS[i])
                    .collect();
                mk_child(
                    "STATIC",
                    &format!("面板顺序：{}", names.join(" → ")),
                    WINDOW_STYLE(0),
                    PAD,
                    k0 + 114,
                    CW,
                    16,
                    IDC_CARDS_HINT,
                );
                mk_child(
                    "STATIC",
                    "",
                    WINDOW_STYLE(SS_ETCHEDHORZ_STYLE),
                    PAD,
                    k0 + 140,
                    CW,
                    2,
                    0,
                );

                // ---- 分组三：其他 ----
                mk_child(
                    "STATIC",
                    "其他",
                    WINDOW_STYLE(0),
                    PAD,
                    o0,
                    200,
                    18,
                    IDC_SEC_OTHER,
                );
                mk_child("STATIC", "主题", WINDOW_STYLE(0), PAD, o0 + 18, 64, 22, 0);
                for (i, label) in THEME_LABELS.iter().enumerate() {
                    // 每组第一颗加 WS_GROUP：自动单选钮按 WS_GROUP 划组，
                    // 否则全窗口的自动单选钮会被当成同一组互相取消（主题与显示方向不能同时选）。
                    mk_child(
                        "BUTTON",
                        label,
                        WINDOW_STYLE(
                            BS_AUTORADIOBUTTON as u32 | if i == 0 { WS_GROUP.0 } else { 0 },
                        ),
                        88 + i as i32 * 60,
                        o0 + 18,
                        56,
                        22,
                        IDC_THEME_BASE + i,
                    );
                }
                let theme_idx = crate::config::THEMES
                    .iter()
                    .position(|v| *v == state.config.capsule.theme)
                    .unwrap_or(0);
                set_check(IDC_THEME_BASE + theme_idx, true);
                mk_child("STATIC", "透明度", WINDOW_STYLE(0), PAD, o0 + 42, 64, 22, 0);
                let opacity_now = state.config.capsule.opacity;
                let opacity_idx = OPACITY_STEPS
                    .iter()
                    .position(|v| (v - opacity_now).abs() < 0.01)
                    .unwrap_or(0);
                for (i, label) in ["100%", "85%", "70%", "55%"].iter().enumerate() {
                    mk_child(
                        "BUTTON",
                        label,
                        WINDOW_STYLE(
                            BS_AUTORADIOBUTTON as u32 | if i == 0 { WS_GROUP.0 } else { 0 },
                        ),
                        88 + i as i32 * 68,
                        o0 + 42,
                        64,
                        22,
                        IDC_OPACITY_BASE + i,
                    );
                }
                set_check(IDC_OPACITY_BASE + opacity_idx, true);
                mk_child(
                    "STATIC",
                    "显示方向",
                    WINDOW_STYLE(0),
                    PAD,
                    o0 + 68,
                    64,
                    22,
                    0,
                );
                for (i, label) in ORIENT_LABELS.iter().enumerate() {
                    mk_child(
                        "BUTTON",
                        label,
                        WINDOW_STYLE(
                            BS_AUTORADIOBUTTON as u32 | if i == 0 { WS_GROUP.0 } else { 0 },
                        ),
                        88 + i as i32 * 60,
                        o0 + 68,
                        56,
                        22,
                        IDC_ORIENT_BASE + i,
                    );
                }
                let orient_idx = crate::config::ORIENTATIONS
                    .iter()
                    .position(|v| *v == state.config.capsule.orientation)
                    .unwrap_or(0);
                set_check(IDC_ORIENT_BASE + orient_idx, true);
                // 展开样式：小面板（320 宽）/ 超大卡片（贴屏幕侧边滑出，尺寸对齐任务栏天气面板）
                mk_child(
                    "STATIC",
                    "展开样式",
                    WINDOW_STYLE(0),
                    PAD,
                    o0 + 94,
                    64,
                    22,
                    0,
                );
                for (i, label) in STYLE_LABELS.iter().enumerate() {
                    mk_child(
                        "BUTTON",
                        label,
                        WINDOW_STYLE(
                            BS_AUTORADIOBUTTON as u32 | if i == 0 { WS_GROUP.0 } else { 0 },
                        ),
                        88 + i as i32 * 76,
                        o0 + 94,
                        72,
                        22,
                        IDC_STYLE_BASE + i,
                    );
                }
                let style_idx = crate::config::EXPAND_STYLES
                    .iter()
                    .position(|v| *v == state.config.capsule.expand_style)
                    .unwrap_or(0);
                set_check(IDC_STYLE_BASE + style_idx, true);
                // 两个勾选并排一行：左「面板自动收起」、右「胶囊/三环切换」
                mk_child(
                    "BUTTON",
                    "面板自动收起",
                    WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                    PAD,
                    o0 + 120,
                    140,
                    22,
                    IDC_CLICK_OUTSIDE_CHK,
                );
                set_check(
                    IDC_CLICK_OUTSIDE_CHK,
                    state.config.capsule.click_outside_collapse,
                );
                // 与托盘右键菜单里的「胶囊/三环切换」是同一个开关，两处都能改
                mk_child(
                    "BUTTON",
                    "胶囊/三环切换",
                    WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                    PAD + 150,
                    o0 + 120,
                    220,
                    22,
                    IDC_TASKBAR_CHK,
                );
                set_check(IDC_TASKBAR_CHK, state.config.capsule.taskbar_pin);
                // 全屏应用（游戏 / 放映）运行时隐藏悬浮窗，退出后自动恢复
                mk_child(
                    "BUTTON",
                    "全屏应用时自动隐藏",
                    WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                    PAD,
                    o0 + 146,
                    260,
                    22,
                    IDC_FS_CHK,
                );
                set_check(IDC_FS_CHK, state.config.capsule.hide_on_fullscreen);

                // ---- 底部按钮（贴客户区底边：竖排展开时设置窗与面板同高，按钮仍要够得着）----
                mk_child("BUTTON", "取消", WINDOW_STYLE(0), 236, btn_y, 80, 30, 2); // IDCANCEL
                mk_child(
                    "BUTTON",
                    "确定",
                    WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
                    324,
                    btn_y,
                    80,
                    30,
                    1,
                ); // IDOK
                LRESULT(0)
            }
            // 分组小标题与卡片提示用系统灰色（其余控件保持默认黑色）
            WM_CTLCOLORSTATIC => {
                let hdc = HDC(wparam.0 as *mut c_void);
                let ctl = HWND(lparam.0 as *mut c_void);
                let _ = SetBkMode(hdc, TRANSPARENT);
                let is_hint = [IDC_SEC_CONN, IDC_SEC_CARDS, IDC_SEC_OTHER, IDC_CARDS_HINT]
                    .iter()
                    .any(|id| GetDlgItem(hwnd, *id as i32).ok() == Some(ctl));
                if is_hint {
                    let _ = SetTextColor(hdc, COLORREF(GetSysColor(COLOR_GRAYTEXT)));
                }
                LRESULT(GetSysColorBrush(COLOR_WINDOW).0 as isize)
            }
            WM_COMMAND => {
                let id = (wparam.0 as u32) & 0xFFFF;
                let code = ((wparam.0 as u32) >> 16) & 0xFFFF;

                // 卡片复选：勾选顺序就是面板里的排列顺序（后勾的排到后面）；
                // 取消掉最后一张时回勾并提示（尽量留住卡片）
                if (IDC_CARD_BASE..IDC_CARD_BASE + CARD_COUNT).contains(&(id as usize))
                    && code == BN_CLICKED_CODE
                {
                    if read_cards(hwnd).any() {
                        if is_checked(hwnd, id as usize) {
                            crate::config::touch_order(
                                &mut state.settings_card_order,
                                id as usize - IDC_CARD_BASE,
                            );
                        }
                        // 回显当前顺序，让"勾一张挪到末尾"看得见
                        let mut cv = read_cards(hwnd);
                        cv.order = state.settings_card_order;
                        let names: Vec<&str> =
                            cv.ordered().iter().map(|&i| CARD_LABELS[i]).collect();
                        set_dlg_text(
                            hwnd,
                            IDC_CARDS_HINT as i32,
                            &format!("面板顺序：{}", names.join(" → ")),
                        );
                    } else {
                        let ctl = GetDlgItem(hwnd, id as i32).unwrap_or_default();
                        let _ = SendMessageW(ctl, BM_SETCHECK, WPARAM(1), LPARAM(0));
                        set_dlg_text(hwnd, IDC_CARDS_HINT as i32, "请至少保留一张卡片");
                    }
                    return LRESULT(0);
                }

                // 透明度 / 明暗主题：点选**即时预览**——只改内存配置并重绘一帧，不落盘。
                // 点"确定"才写盘生效；点"取消"或直接关窗由 WM_NCDESTROY 用快照还原。
                let is_opacity = (IDC_OPACITY_BASE..IDC_OPACITY_BASE + OPACITY_STEPS.len())
                    .contains(&(id as usize));
                let is_theme =
                    (IDC_THEME_BASE..IDC_THEME_BASE + THEME_LABELS.len()).contains(&(id as usize));
                if (is_opacity || is_theme) && code == BN_CLICKED_CODE {
                    apply_settings_preview(state, hwnd);
                    return LRESULT(0);
                }

                if id == 1 {
                    // IDOK：地址非法只拦地址本身，其余开关照常保存并立即生效
                    let text = get_dlg_text(hwnd, IDC_ADDR_EDIT as i32);
                    let token_text = get_dlg_text(hwnd, IDC_TOKEN_EDIT as i32);
                    let mut nas = state.config.nas.clone();
                    let addr_ok = nas.set_addr(&text);
                    if addr_ok {
                        nas.token = token_text.trim().to_string();
                        state.config.nas = nas;
                    }

                    let was_pinned = state.config.capsule.taskbar_pin;
                    let theme_idx = (0..THEME_LABELS.len())
                        .find(|&i| is_checked(hwnd, IDC_THEME_BASE + i))
                        .unwrap_or(0);
                    state.config.capsule.theme = crate::config::THEMES[theme_idx].to_string();
                    state.config.capsule.taskbar_pin = is_checked(hwnd, IDC_TASKBAR_CHK);
                    state.config.capsule.opacity = read_opacity(hwnd);
                    state.config.capsule.click_outside_collapse =
                        is_checked(hwnd, IDC_CLICK_OUTSIDE_CHK);
                    state.config.capsule.hide_on_fullscreen = is_checked(hwnd, IDC_FS_CHK);
                    let orient_idx = (0..ORIENT_LABELS.len())
                        .find(|&i| is_checked(hwnd, IDC_ORIENT_BASE + i))
                        .unwrap_or(0);
                    state.config.capsule.orientation =
                        crate::config::ORIENTATIONS[orient_idx].to_string();
                    let was_big = state.config.capsule.expand_big();
                    let style_idx = (0..STYLE_LABELS.len())
                        .find(|&i| is_checked(hwnd, IDC_STYLE_BASE + i))
                        .unwrap_or(0);
                    state.config.capsule.expand_style =
                        crate::config::EXPAND_STYLES[style_idx].to_string();
                    // 展开样式只影响展开形态：切换时先收起，避免旧面板几何残留
                    if was_big != state.config.capsule.expand_big() && state.view.expanded() {
                        collapse(state);
                    }
                    let mut picked = read_cards(hwnd);
                    // 兜底：手改配置把六项全关掉时，按"至少保留一张卡片"回到全开
                    if !picked.any() {
                        picked = CardVis::default();
                    } else {
                        // 顺序取设置窗里记录的勾选先后
                        picked.order = state.settings_card_order;
                    }
                    state.config.capsule.cards = picked;

                    // 方向变了：先落到该方向的默认位置（竖条贴右缘 / 横条贴顶边），
                    // 免得横竖互相"卡"在对方的位置上；再统一应用布局
                    let want_v = state.config.capsule.orientation_hint() == Some(true);
                    let cur_v = match state.shared.read() {
                        Ok(g) => g.vertical,
                        Err(e) => {
                            eprintln!("[App] RwLock poisoned in settings_wndproc cur_v: {:?}", e);
                            return LRESULT(0);
                        }
                    };
                    if want_v != cur_v {
                        let (cw, _) = collapsed_main_size(want_v);
                        let (dx, dy) = default_capsule_pos(cw as i32, want_v);
                        state.config.capsule.x = Some(dx);
                        state.config.capsule.y = Some(dy);
                        let _ = SetWindowPos(
                            state.hwnd,
                            None,
                            dx,
                            dy,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                    let _ = crate::config::save(&state.config);

                    // "点击面板外自动收起"即时生效：面板正开着就立刻挂/撤全局鼠标钩子
                    if state.view.expanded() {
                        if state.config.capsule.click_outside_collapse {
                            mouse_hook_install();
                        } else {
                            mouse_hook_remove();
                        }
                    }

                    // 立即生效：主题/掩码/主机名同步给渲染侧并强制重绘一帧
                    {
                        let mut sh = match state.shared.write() {
                            Ok(g) => g,
                            Err(e) => {
                                eprintln!("[App] RwLock poisoned in settings_wndproc final sync: {:?}", e);
                                return LRESULT(0);
                            }
                        };
                        sh.nas_host = state.config.nas.host.clone();
                        sh.dark = state.config.capsule.theme_dark();
                        sh.cards = state.config.capsule.cards;
                        sh.big = state.config.capsule.expand_big();
                        sh.bump_seq();
                    }
                    if state.config.capsule.taskbar_pin != was_pinned {
                        apply_taskbar_pin(state);
                    } else {
                        // 方向/位置变化在这里落定，再强制重绘一帧
                        apply_layout(state, true);
                        resize_window(state);
                        paint_frame(state, true);
                    }

                    // 刚关掉"全屏应用时自动隐藏"时，若此前正被全屏藏起来，这里立刻恢复
                    sync_fullscreen_state(state);

                    // 已经落盘生效，快照作废：之后关窗不该再"还原"
                    state.settings_orig = None;
                    if addr_ok {
                        let _ = state.cmd_tx.send(SourceCmd::SetTarget(
                            state.config.nas.addr(),
                            state.config.nas.token.clone(),
                        ));
                        let _ = DestroyWindow(hwnd);
                    } else {
                        // 只拦地址：窗口留着让用户改地址，其余设置已经保存并生效
                        set_dlg_text(hwnd, IDC_TEST_RESULT as i32, "地址格式无效，连接设置未保存");
                    }
                } else if id == 2 {
                    // IDCANCEL
                    let _ = DestroyWindow(hwnd);
                } else if id == IDC_TEST_BUTTON as u32 {
                    let text = get_dlg_text(hwnd, IDC_ADDR_EDIT as i32);
                    let token_text = get_dlg_text(hwnd, IDC_TOKEN_EDIT as i32);
                    let mut nas = state.config.nas.clone();
                    if !nas.set_addr(&text) {
                        set_dlg_text(hwnd, IDC_TEST_RESULT as i32, "地址格式无效");
                    } else {
                        set_dlg_text(hwnd, IDC_TEST_RESULT as i32, "测试中…");
                        let token = token_text.trim().to_string();
                        let hwnd_isize = hwnd.0 as isize;
                        std::thread::spawn(move || {
                            let msg: &'static str = match test_ws(&nas.addr(), &token) {
                                Ok(()) => "连接成功",
                                Err(e) => Box::leak(e.into_boxed_str()),
                            };
                            if let Ok(ctl) =
                                GetDlgItem(HWND(hwnd_isize as *mut _), IDC_TEST_RESULT as i32)
                            {
                                let wide: Vec<u16> = msg.encode_utf16().chain(Some(0)).collect();
                                let _ = SetWindowTextW(ctl, PCWSTR(wide.as_ptr()));
                            }
                        });
                    }
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_NCDESTROY => {
                // 窗口没了但快照还在 = 用户是"取消/直接关窗"退出的，
                // 把即时预览过的主题与透明度还原回打开设置窗时的样子
                restore_settings_preview(state);
                state.settings_hwnd = HWND(null_mut());
                // 释放设置窗字体（每开一次窗建一个，关窗即回收）
                if !state.settings_font.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(state.settings_font.0));
                    state.settings_font = HFONT(null_mut());
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe extern "system" fn settings_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = lparam.0 as *const CREATESTRUCTW;
        if !cs.is_null() {
            let ptr = (*cs).lpCreateParams as isize;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr);
        }
        return LRESULT(1);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if ptr == 0 {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *(ptr as *mut AppState);
    settings_wndproc_impl(state, hwnd, msg, wparam, lparam)
}

fn get_dlg_text(hwnd: HWND, id: i32) -> String {
    unsafe {
        let ctl = GetDlgItem(hwnd, id).unwrap_or_default();
        let len = GetWindowTextLengthW(ctl) as usize;
        let mut buf = vec![0u16; len + 1];
        GetWindowTextW(ctl, &mut buf);
        String::from_utf16_lossy(&buf[..len])
    }
}

fn set_dlg_text(hwnd: HWND, id: i32, text: &str) {
    unsafe {
        if let Ok(ctl) = GetDlgItem(hwnd, id) {
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let _ = SetWindowTextW(ctl, PCWSTR(wide.as_ptr()));
        }
    }
}

/// 临时连接 NAS /ws 验证地址与令牌。成功则关闭 socket 返回 Ok；
/// 失败返回简短中文原因，用于设置窗“测试连接”提示。
fn test_ws(addr: &str, token: &str) -> Result<(), String> {
    use std::io::ErrorKind;
    let url = format!("ws://{addr}/ws?token={token}");
    match tungstenite::connect(&url) {
        Ok((mut ws, _)) => {
            let _ = ws.close(None);
            Ok(())
        }
        Err(tungstenite::Error::Http(resp)) => {
            let code = resp.status().as_u16();
            if code == 401 || code == 403 {
                Err("配对令牌错误".into())
            } else {
                Err(format!("服务器返回 {code}"))
            }
        }
        Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::ConnectionRefused => {
            Err("无法连接到 NAS，请检查地址".into())
        }
        Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::TimedOut => {
            Err("连接超时，请检查地址或网络".into())
        }
        Err(_) => Err("连接失败，请检查地址和令牌".into()),
    }
}
