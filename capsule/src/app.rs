//! 主窗口：无边框置顶、层叠透明 DIB、系统托盘。
//!
//! 架构：
//!   - 1 个 DWM 顶层窗口（WS_POPUP、WS_EX_TOPMOST、WS_EX_LAYERED），
//!     全部绘制由 tiny-skia → DIB → UpdateLayeredWindow 完成；
//!   - 鼠标拖动：WM_LBUTTONDOWN 捕获 → WM_MOUSEMOVE 移动 → WM_LBUTTONUP；
//!     移动距离 < 4px 视为点击，触发展开/收起；
//!   - 展开时 SetActiveWindow 以接收 Esc / WM_KILLFOCUS（点击外部收起）。

use crate::config::Config;
use crate::data::*;
use crate::font::Font;
use crate::ui::capsule_view::{self, CapsuleView};
use crate::ui::View;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::raw::c_void;
use std::ptr::null_mut;
use std::sync::mpsc::channel;
use std::sync::{Arc, RwLock};
use std::time::Instant;

#[allow(dead_code)]
fn dbg_log(s: &str) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let _ = OpenOptions::new().create(true).append(true).open("C:/Users/nmche/AppData/Local/Temp/htc_debug.log")
        .and_then(|mut f| writeln!(f, "{}.{:03} {}", t.as_secs() % 10000, t.subsec_millis(), s));
}
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Networking::WinSock::{WSAStartup, WSADATA};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

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
        Box::leak(s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>().into_boxed_slice())
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
    // 串行预热后该问题消失。fake 源不用网络，不初始化。
    if config.nas.source != "fake" {
        unsafe {
            let mut wsa: WSADATA = std::mem::zeroed();
            let _ = WSAStartup(0x0202, &mut wsa);
        }
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
    unsafe { let _ = RegisterClassW(&wc); }

    // 任务栏内三环子窗口使用独立的窗口类（不同 wndproc）。
    let wc_rings = WNDCLASSW {
        lpfnWndProc: Some(rings_wndproc),
        hInstance: instance.into(),
        lpszClassName: rings_class_name(),
        hCursor: unsafe { LoadCursorW(None, IDC_HAND).unwrap_or_default() },
        hbrBackground: HBRUSH(null_mut()),
        ..Default::default()
    };
    unsafe { let _ = RegisterClassW(&wc_rings); }

    let shared = Arc::new(RwLock::new(SharedState::default()));
    shared.write().unwrap().nas_host = config.nas.host.clone();
    let (cmd_tx, cmd_rx) = channel::<SourceCmd>();

    let hwnd = create_window(instance, (initial_w(&config), CAPSULE_H));
    if hwnd.is_invalid() {
        return;
    }
    set_initial_pos(hwnd, &config);

    tray_init(hwnd, app_icons().1);

    // 磨砂配置启动生效
    if config.capsule.frost {
        shared.write().unwrap().frost = true;
        unsafe { apply_frost(hwnd, true); }
    }

    let view = Box::new(CapsuleView::new(Arc::new(std::sync::atomic::AtomicIsize::new(
        hwnd.0 as isize,
    ))));
    let font = Font::load();
    // 预热界面用字字形缓存（启动期一次性冷光栅化，避免展开首帧毫秒级尖峰）
    if let Some(f) = &font {
        f.preheat();
    }
    let addr = config.nas.addr();
    let token = config.nas.token.clone();
    let source_kind = config.nas.source.clone();

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
        last_seq: 0,
        pin_anchor: None,
        pin_hidden: false,
        rings: None,
        acrylic: None,
        seg: None,
        seg_rect: None,
        seg_at: None,
    };

    let raw = Box::into_raw(Box::new(state));
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize); }

    // 首次运行或配置不完整：自动打开设置窗，避免用户看到空白胶囊不知所措。
    // fake 模式用于演示，不弹窗。
    if source_kind != "fake" && unsafe { (*raw).config.nas.needs_setup() } {
        unsafe { open_settings(&mut *raw); }
    }

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
    }
    paint_frame(unsafe { &mut *raw }, true);

    // 启动时若已开启“固定在任务栏”，立即归位并开始保持置顶
    if unsafe { (*raw).config.capsule.taskbar_pin } {
        apply_taskbar_pin(unsafe { &mut *raw });
    }

    // 数据源在窗口首帧完成后再启动：配合入口处的 WSAStartup 串行预热，
    // 双重保证 winsock 首次连接不与窗口/合成器初始化并发（否则系统侧会
    // 出现约 68MB 的异常内存提交，实测概率性触发）。
    let source_handle = match source_kind.as_str() {
        "fake" => FakeSource.spawn(SourceCtx {
            hwnd: hwnd.0 as isize,
            state: shared.clone(),
            cmd_rx,
            target: addr,
            token,
        }),
        _ => WsSource.spawn(SourceCtx {
            hwnd: hwnd.0 as isize,
            state: shared.clone(),
            cmd_rx,
            target: addr,
            token,
        }),
    };

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
unsafe extern "system" fn acrylic_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
        unsafe { let _ = RegisterClassW(&wc); }
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
        unsafe { let _ = GetWindowRect(a.hwnd, &mut rc); }
        if rc.left == ax && rc.top == ay && (rc.right - rc.left) == aw && (rc.bottom - rc.top) == ah {
            return;
        }
        unsafe {
            // 亚克力窗口在三环/面板之下，Z-order 低于 HWND_TOPMOST
            let _ = SetWindowPos(
                a.hwnd,
                HWND_NOTOPMOST,
                ax,
                ay,
                aw,
                ah,
                SWP_NOACTIVATE,
            );
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
    let Some(tb) = shell_tray_wnd() else { return false };
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
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as *mut AppState as isize); }
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
        unsafe { let _ = GetWindowRect(r.hwnd, &mut rc); }
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
    let Some(r) = state.rings.as_mut() else { return };
    let seq = state.shared.read().unwrap().seq();
    if !force && seq == r.last_seq {
        return;
    }
    r.last_seq = seq;
    let frame_data = state.shared.read().unwrap().grab();
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
    capsule_view::draw_rings(&mut r.pix, &frame_data.as_frame(), state.font.as_ref());
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

/// 三环窗口拖动跟手：横向限制在任务栏可用空段内。
fn rings_drag_move(state: &mut AppState) {
    let Some((x0, x1)) = taskbar_free_segment(state) else { return };
    let Some(grab) = state.rings.as_ref().map(|r| r.drag_grab) else { return };
    let mut pt = POINT::default();
    unsafe { let _ = GetCursorPos(&mut pt); }
    let ax = pin_fit(x0, x1, Some(pt.x - grab));
    state.pin_anchor = Some((ax, ring_anchor_y(), true));
    rings_place(state, ax, ring_anchor_y());
}

/// 三环窗口拖动松手：记住选定位置。
fn rings_drag_commit(state: &mut AppState) {
    let Some((x0, x1)) = taskbar_free_segment(state) else { return };
    let Some(r) = state.rings.as_ref() else { return };
    let mut wr = RECT::default();
    unsafe { let _ = GetWindowRect(r.hwnd, &mut wr); }
    let ax = pin_fit(x0, x1, Some(wr.left));
    state.config.capsule.taskbar_x = Some(ax);
    let _ = crate::config::save(&state.config);
    state.pin_anchor = Some((ax, ring_anchor_y(), true));
    rings_place(state, ax, ring_anchor_y());
}

/// 任务栏内三环子窗口的消息处理：点击展开/收起面板，拖动在任务栏内换位。
unsafe extern "system" fn rings_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
            unsafe { let _ = SetCapture(hwnd); }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let moved = match state.rings.as_mut() {
                Some(r) if r.dragging => {
                    let mut pt = POINT::default();
                    unsafe { let _ = GetCursorPos(&mut pt); }
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
            unsafe { let _ = ReleaseCapture(); }
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
    unsafe {
        CreateWindowExW(
            ex_style,
            class_name(),
            window_title(),
            style,
            CW_USEDEFAULT, CW_USEDEFAULT,
            size.0 as i32, size.1 as i32,
            None,
            None,
            instance,
            None,
        )
        .unwrap_or_default()
    }
}

/// 启动时的收起态宽度：固定在任务栏用紧凑宽度，否则为药丸胶囊宽度。
fn initial_w(config: &Config) -> u32 {
    if config.capsule.taskbar_pin {
        CAPSULE_W
    } else {
        CapsuleView::pill_width()
    }
}

/// 初始位置：配置优先，否则工作区右上角。
fn set_initial_pos(hwnd: HWND, config: &Config) {
    let (mut x, mut y) = match (config.capsule.x, config.capsule.y) {
        (Some(x), Some(y)) => (x, y),
        _ => {
            let mut rc = RECT::default();
            unsafe {
                let _ = SystemParametersInfoW(
                    SPI_GETWORKAREA,
                    0,
                    Some(&mut rc as *mut RECT as *mut c_void),
                    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                );
            }
            (rc.right - initial_w(config) as i32 - 24, rc.top + 24)
        }
    };
    if x < 0 {
        x = 24;
    }
    if y < 0 {
        y = 24;
    }
    unsafe {
        let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

fn tray_init(hwnd: HWND, icon: HICON) {
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
    unsafe { let _ = Shell_NotifyIconW(NIM_ADD, &nid); }
}

fn tray_remove(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    unsafe { let _ = Shell_NotifyIconW(NIM_DELETE, &nid); }
}

const WM_APP_TRAY: u32 = WM_APP + 2;
const ID_TRAY_SETTINGS: u32 = 1001;
const ID_TRAY_EXIT: u32 = 1002;
const ID_TRAY_OPACITY_BASE: u32 = 2001;
const ID_TRAY_FROST: u32 = 2005;
const ID_TRAY_TASKBAR: u32 = 2006;
/// 展开/收起动效定时器 ID
const IDT_ANIM: usize = 0x4A11;
/// 固定在任务栏时，周期性把窗口重新提到任务栏之上的定时器 ID
const IDT_PIN: usize = 0x4A12;
/// 托盘可选择的透明度档位
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
                    unsafe { let _ = ShowWindow(state.hwnd, SW_HIDE); }
                    acrylic_show(state, false);
                }
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == IDT_PIN => {
            // 周期重估任务栏空间：任务栏内容变化（天气/小组件开关）时重新选空段并归位三环。
            if state.config.capsule.taskbar_pin {
                sync_taskbar_state(state);
            } else {
                unsafe { let _ = KillTimer(hwnd, IDT_PIN); }
            }
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
            // 固定到任务栏时主窗口只作展开面板显示，不参与拖动（收起态由任务栏内三环窗口拖动换位）
            if state.config.capsule.taskbar_pin {
                return LRESULT(0);
            }
            if state.dragging {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                if (pt.x - state.drag_origin.0).abs() >= 4 || (pt.y - state.drag_origin.1).abs() >= 4 {
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
                let _ = SetWindowPos(hwnd, None, nx, ny, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
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
                        open_settings(state);
                    } else {
                        toggle_expand(state);
                    }
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
                let over_own = cursor_over_window(state.hwnd)
                    || state.rings.as_ref().is_some_and(|r| cursor_over_window(r.hwnd));
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
        WM_COMMAND => {
            let id = wparam.0 as u32;
            if id == ID_TRAY_EXIT {
                let _ = state.cmd_tx.send(SourceCmd::Shutdown);
                tray_remove(state.hwnd);
                PostQuitMessage(0);
            } else if id == ID_TRAY_SETTINGS {
                open_settings(state);
            } else if (ID_TRAY_OPACITY_BASE..ID_TRAY_OPACITY_BASE + OPACITY_STEPS.len() as u32).contains(&id) {
                state.config.capsule.opacity = OPACITY_STEPS[(id - ID_TRAY_OPACITY_BASE) as usize];
                let _ = crate::config::save(&state.config);
                paint_frame(state, true);
            } else if id == ID_TRAY_FROST {
                state.config.capsule.frost = !state.config.capsule.frost;
                let _ = crate::config::save(&state.config);
                state.shared.write().unwrap().frost = state.config.capsule.frost;
                unsafe { apply_frost(state.hwnd, state.config.capsule.frost); }
                paint_frame(state, true);
            } else if id == ID_TRAY_TASKBAR {
                state.config.capsule.taskbar_pin = !state.config.capsule.taskbar_pin;
                let _ = crate::config::save(&state.config);
                apply_taskbar_pin(state);
            }
            LRESULT(0)
        }
        WM_DESTROY | WM_CLOSE => {
            dbg_log("WM_DESTROY/CLOSE");
            let _ = state.cmd_tx.send(SourceCmd::Shutdown);
            tray_remove(state.hwnd);
            rings_destroy(state);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
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

fn collapse(state: &mut AppState) {
    if !state.view.expanded() {
        return;
    }
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
    let seq = state.shared.read().unwrap().seq();
    if !force && seq == state.last_seq {
        return;
    }
    state.last_seq = seq;

    let frame_data = state.shared.read().unwrap().grab();
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
    let presented = state.view.render(&mut state.pix, &frame_data.as_frame(), state.font.as_ref(), (vis_w, vis_h));
    if !presented {
        // 视图判定本帧画面无变化（稳态数据帧已派发后台构建，等 WM_APP_PANEL
        // 携带新面板再上屏）：跳过像素拷贝与 UpdateLayeredWindow 提交。
        return;
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
                    *e = (v as f32 * k) as u8;
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
    let pt_dst = POINT { x: rc.left, y: rc.top };
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
    let changed = state.view.expanded() != open;
    if changed {
        state.view.set_expanded(open);
        let detail = if open { Detail::Expanded } else { Detail::Collapsed };
        let _ = state.cmd_tx.send(SourceCmd::SetDetail(detail));
    }
    let pinned = state.config.capsule.taskbar_pin;
    if pinned && !changed {
        return; // 固定态已处于目标状态，无需重播动画
    }
    if open {
        // 展开前先把窗口摆到位（几何变化在隐藏态完成，避免显示瞬间闪现旧位置）
        resize_window(state);
        unsafe { let _ = ShowWindow(state.hwnd, SW_SHOWNA); }
        // 磨砂背景层与面板同显（固定态）
        if pinned {
            acrylic_show(state, true);
        }
    }
    if pinned {
        if open {
            unsafe { let _ = SetActiveWindow(state.hwnd); }
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

/// 按当前动画进度重设窗口；宽度变化时保持右缘不动（向左展开/收回）。
/// 固定在任务栏时：面板在任务栏内三环上水平居中；任务栏在下方则面板向上生长（底边贴任务栏顶）。
fn resize_window(state: &mut AppState) {
    let frame_data = state.shared.read().unwrap().grab();
    let frame = frame_data.as_frame();
    let (w, h) = state.view.frame_size(&frame);
    // 固定态：亚克力层按“可见面板高度”跟随（底/顶边与面板一致，随内容一起生长/收回）
    let vis_h = state.view.visible_panel_h(&frame) as i32;
    let up = frame.expand_up;
    unsafe {
        let mut rc = RECT::default();
        let _ = GetWindowRect(state.hwnd, &mut rc);
        let (x, y) = match (state.config.capsule.taskbar_pin, state.pin_anchor) {
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
                    taskbar_rect().map(|t| t.top).unwrap_or(py + CAPSULE_H as i32) - h as i32
                } else {
                    taskbar_rect().map(|t| t.bottom).unwrap_or(py)
                };
                (x, y)
            }
            _ => (rc.right - w as i32, rc.top),
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
        if state.config.capsule.taskbar_pin {
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
    let tb = unsafe { FindWindowW(static_pcwstr("Shell_TrayWnd"), PCWSTR::null()) }.unwrap_or_default();
    if let Ok(h) = unsafe { FindWindowExW(tb, None, static_pcwstr("TrayNotifyWnd"), PCWSTR::null()) } {
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
    let tb = unsafe { FindWindowW(static_pcwstr("Shell_TrayWnd"), PCWSTR::null()) }.unwrap_or_default();
    // ReBarWindow32 是任务按钮区的父容器；Win11 上其右缘即已打开窗口按钮区右缘。
    if let Ok(h) = unsafe { FindWindowExW(tb, None, static_pcwstr("ReBarWindow32"), PCWSTR::null()) } {
        if !h.0.is_null() {
            let mut r = RECT::default();
            if unsafe { GetWindowRect(h, &mut r) }.is_ok() && r.right > tr.left && r.right < tr.right {
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

/// 任务栏条带像素扫描找“视觉空段”。
/// Win11 任务栏右侧的按钮/小组件由 XAML 合成、拿不到子窗口、UIA 也枚举不出元素，
/// 因此直接看像素：空白列颜色均匀，有内容的列色差大。
/// 返回 [任务栏左缘, 托盘左缘) 内能放下三环的最大连续空段。
fn pixels_free_segment(state: &AppState, tr: &RECT) -> Option<(i32, i32)> {
    let tw = tr.right - tr.left;
    let th = tr.bottom - tr.top;
    if tw <= 8 || th <= 8 {
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
    let bmp = unsafe { CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, HANDLE(null_mut()), 0) }
        .unwrap_or_default();
    let seg = if bmp.0.is_null() || bits.is_null() {
        None
    } else {
        unsafe {
            let old = SelectObject(mem, HGDIOBJ(bmp.0));
            let screen = GetDC(HWND(null_mut()));
            let ok = BitBlt(mem, 0, 0, tw, th, screen, tr.left, tr.top, SRCCOPY).is_ok();
            let _ = ReleaseDC(HWND(null_mut()), screen);
            let out = if ok { scan_free_columns(state, tr, bits, tw, th) } else { None };
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
    seg
}

/// 逐列判定“是否为空”，取最大连续空段。自身三环窗口覆盖的列按空处理（它本来就在空段里）。
fn scan_free_columns(state: &AppState, tr: &RECT, bits: *const c_void, tw: i32, th: i32) -> Option<(i32, i32)> {
    let stride = tw as usize * 4;
    let data = unsafe { std::slice::from_raw_parts(bits as *const u8, stride * th as usize) };
    // 只在托盘左侧搜索，并留出托盘溢出按钮（^）的余量
    let w = (taskbar_tray_left(tr) - tr.left - TRAY_MARGIN).clamp(0, tw) as usize;
    if w == 0 {
        return None;
    }
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
    // 自身窗口覆盖的列不参与判定
    if let Some(r) = state.rings.as_ref() {
        let mut rc = RECT::default();
        unsafe { let _ = GetWindowRect(r.hwnd, &mut rc); }
        let l = (rc.left - tr.left).clamp(0, w as i32) as usize;
        let rr = (rc.right - tr.left).clamp(0, w as i32) as usize;
        if l < rr {
            free[l..rr].fill(true);
        }
    }
    // 最大连续空段
    let mut best: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < w {
        if free[i] {
            let s = i;
            while i < w && free[i] {
                i += 1;
            }
            if best.is_none_or(|(bs, be)| i - s > be - bs) {
                best = Some((s, i));
            }
        } else {
            i += 1;
        }
    }
    let (s, e) = best?;
    let seg = (tr.left + s as i32, tr.left + e as i32);
    (seg.1 - seg.0 >= PIN_NEED).then_some(seg)
}

/// 三环窗口当前左缘（屏幕坐标）。用于定位滞回：任务栏透明时像素判定会随壁纸噪声抖动，
/// 若每次都吸附到空段左缘，三环会来回跑；以当前位置为首选即可保持稳定。
fn rings_current_x(state: &AppState) -> Option<i32> {
    if state.pin_hidden {
        return None;
    }
    let r = state.rings.as_ref()?;
    let mut rc = RECT::default();
    unsafe { let _ = GetWindowRect(r.hwnd, &mut rc); }
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
    if let Some(seg) = pixels_free_segment(state, tr) {
        return Some(seg);
    }
    // 兜底：紧靠托盘左侧（任务按钮与托盘之间通常为空）
    let right = taskbar_tray_left(tr);
    let left = (right - PIN_GAP - CAPSULE_W as i32).max(taskbar_rebar_right(tr) + PIN_GAP);
    (right - left >= CAPSULE_W as i32).then_some((left, right))
}

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
    unsafe { let _ = ReleaseDC(HWND(null_mut()), dc); }
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
                if let Some((x0, x1)) = taskbar_free_segment(state) {
                    fits = true;
                    // 定位优先级：用户拖动位置 > 当前位置（滞回，防透明任务栏下空段判定抖动导致乱跑）> 空段左缘
                    let cur = rings_current_x(state);
                    let prefer = state.config.capsule.taskbar_x.or(cur);
                    let mut ax = pin_fit(x0, x1, prefer);
                    if let Some(cx) = cur {
                        if (ax - cx).abs() < PIN_DEADZONE {
                            ax = cx; // 差异极小：保持原位，不做微调
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
        let mut sh = state.shared.write().unwrap();
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
                unsafe { let _ = ShowWindow(r.hwnd, SW_HIDE); }
            }
        }
        state.pin_hidden = !fits;
        unsafe { let _ = ShowWindow(state.hwnd, SW_HIDE); }
        // 收起态不显示磨砂背景层（否则桌面残留无内容灰底）
        acrylic_show(state, false);
    } else {
        rings_destroy(state);
        state.pin_hidden = false;
        unsafe { let _ = ShowWindow(state.hwnd, SW_SHOWNA); }
        if dirty {
            resize_window(state);
            paint_frame(state, true);
        }
    }
}

/// 应用/取消“固定在任务栏”：三环挂成任务栏的从属窗口常驻其上方，展开面板为独立浮窗。
fn apply_taskbar_pin(state: &mut AppState) {
    if !state.config.capsule.taskbar_pin {
        state.pin_anchor = None;
        state.view.set_hovered(false);
        unsafe { let _ = KillTimer(state.hwnd, IDT_PIN); }
        // 取消固定：销毁任务栏内的三环子窗口，收起面板，恢复普通胶囊
        state.view.set_expanded(false);
        state.view.jump_anim();
        sync_taskbar_state(state); // 恢复普通布局与配色，并显示主窗口
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
    let sub = unsafe { CreatePopupMenu().unwrap() };
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, ID_TRAY_SETTINGS as usize, pcwstr("设置 NAS 地址"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

        // 透明度子菜单
        let labels = ["100%", "85%", "70%", "55%"];
        for (i, label) in labels.iter().enumerate() {
            let mut flags = MF_STRING;
            if (state.config.capsule.opacity - OPACITY_STEPS[i]).abs() < 0.01 {
                flags |= MF_CHECKED;
            }
            let _ = AppendMenuW(sub, flags, (ID_TRAY_OPACITY_BASE + i as u32) as usize, pcwstr(label));
        }
        let _ = AppendMenuW(menu, MF_POPUP | MF_STRING, sub.0 as usize, pcwstr("透明度"));

        let frost_flags = if state.config.capsule.frost { MF_STRING | MF_CHECKED } else { MF_STRING };
        let _ = AppendMenuW(menu, frost_flags, ID_TRAY_FROST as usize, pcwstr("磨砂效果"));

        let pin_flags = if state.config.capsule.taskbar_pin { MF_STRING | MF_CHECKED } else { MF_STRING };
        let _ = AppendMenuW(menu, pin_flags, ID_TRAY_TASKBAR as usize, pcwstr("固定在任务栏"));
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

// ---- 磨砂：SetWindowCompositionAttribute（未公开 API，Win10 1803+ 可用）----

#[repr(C)]
struct AccentPolicy {
    accent_state: u32,
    accent_flags: u32,
    /// AABBGGRR
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct WindowCompositionAttribData {
    attrib: u32, // WCA_ACCENT_POLICY
    data: *mut c_void,
    size_of_data: usize,
}

const WCA_ACCENT_POLICY: u32 = 19;
const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

/// 开/关磨砂。失败时静默返回（老系统没有该 API）。
unsafe fn apply_frost(hwnd: HWND, on: bool) {
    let user32 = GetModuleHandleW(pcwstr("user32.dll")).unwrap_or_default();
    let Some(far) = GetProcAddress(user32, pcstr("SetWindowCompositionAttribute")) else {
        return;
    };
    type SetWca = unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> BOOL;
    let set_wca: SetWca = std::mem::transmute(far);
    let mut policy = AccentPolicy {
        accent_state: if on { ACCENT_ENABLE_ACRYLICBLURBEHIND } else { ACCENT_DISABLED },
        accent_flags: 0,
        // 深色薄纱，与胶囊底色一致
        gradient_color: 0x5A1E1412,
        animation_id: 0,
    };
    let mut data = WindowCompositionAttribData {
        attrib: WCA_ACCENT_POLICY,
        data: &mut policy as *mut AccentPolicy as *mut c_void,
        size_of_data: std::mem::size_of::<AccentPolicy>(),
    };
    let _ = set_wca(hwnd, &mut data);
}

// ---- 设置窗口：NAS 地址输入 ----

const SETTINGS_CLASS: &str = "HeTaoCapsuleSettings";
const IDC_ADDR_EDIT: usize = 101;
const IDC_TOKEN_EDIT: usize = 102;
const IDC_TEST_BUTTON: usize = 103;
const IDC_TEST_RESULT: usize = 104;
const IDC_TASKBAR_CHK: usize = 105;

fn open_settings(state: &mut AppState) {
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
        unsafe { let _ = RegisterClassW(&wc); }
    });

    // 以胶囊/面板位置为参照：优先贴其左侧、底边对齐；放不下改右侧；最后夹进工作区，保证整窗可见
    // （固定到任务栏时面板底边已是任务栏顶，不能再用「面板下方」定位）
    let mut rc = RECT::default();
    unsafe { let _ = GetWindowRect(state.hwnd, &mut rc); }
    let mut wa = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut wa as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    let (w, h) = (360, 288);
    let gap = 12;
    let max_x = (wa.right - w).max(wa.left);
    let max_y = (wa.bottom - h).max(wa.top);
    let mut x = rc.left - w - gap;
    if x < wa.left {
        x = rc.right + gap;
    }
    let x = x.clamp(wa.left, max_x);
    let y = (rc.bottom - h).clamp(wa.top, max_y);

    let class_wide: Vec<u16> = SETTINGS_CLASS.encode_utf16().chain(Some(0)).collect();
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_TOPMOST,
            PCWSTR(class_wide.as_ptr()),
            pcwstr("设置 NAS 连接"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            x, y, w, h,
            state.hwnd,
            None,
            instance,
            Some(state as *mut AppState as *const c_void),
        )
        .unwrap_or_default()
    };
    state.settings_hwnd = hwnd;
}

fn child_font(hwnd: HWND) {
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    unsafe {
        let _ = SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    }
}

fn settings_wndproc_impl(state: &mut AppState, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let mk_child = |class: &str, text: &str, style: WINDOW_STYLE, x: i32, y: i32, w: i32, h: i32, id: usize| {
                    let h = CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        pcwstr(class),
                        pcwstr(text),
                        WS_CHILD | WS_VISIBLE | style,
                        x, y, w, h,
                        hwnd,
                        HMENU(id as *mut c_void),
                        GetModuleHandleW(None).unwrap_or_default(),
                        None,
                    )
                    .unwrap_or_default();
                    child_font(h);
                    h
                };
                mk_child("STATIC", "NAS 地址（host:port）：", WINDOW_STYLE(0), 12, 10, 330, 20, 100);
                let edit = mk_child(
                    "EDIT",
                    &state.config.nas.addr(),
                    WINDOW_STYLE(WS_BORDER.0 | ES_AUTOHSCROLL as u32),
                    12, 34, 324, 24,
                    IDC_ADDR_EDIT,
                );
                let _ = edit;
                mk_child("STATIC", "配对令牌（见 NAS 管理页）：", WINDOW_STYLE(0), 12, 64, 330, 20, 103);
                let token_edit = mk_child(
                    "EDIT",
                    &state.config.nas.token,
                    WINDOW_STYLE(WS_BORDER.0 | ES_AUTOHSCROLL as u32),
                    12, 88, 324, 24,
                    IDC_TOKEN_EDIT,
                );
                let _ = token_edit;
                mk_child("BUTTON", "测试连接", WINDOW_STYLE(0), 12, 120, 90, 26, IDC_TEST_BUTTON);
                mk_child("STATIC", "", WINDOW_STYLE(0), 110, 150, 226, 20, IDC_TEST_RESULT);
                let chk = mk_child(
                    "BUTTON",
                    "固定在任务栏（显示在任务栏最左端）",
                    WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                    12, 180, 324, 22,
                    IDC_TASKBAR_CHK,
                );
                if state.config.capsule.taskbar_pin {
                    // 1 = BST_CHECKED（勾选态）
                    let _ = SendMessageW(chk, BM_SETCHECK, WPARAM(1), LPARAM(0));
                }
                mk_child("BUTTON", "确定", WINDOW_STYLE(BS_DEFPUSHBUTTON as u32), 170, 216, 80, 26, 1); // IDOK
                mk_child("BUTTON", "取消", WINDOW_STYLE(0), 258, 216, 80, 26, 2); // IDCANCEL
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = wparam.0 as u32 & 0xFFFF;
                if id == 1 {
                    // IDOK
                    let text = get_dlg_text(hwnd, IDC_ADDR_EDIT as i32);
                    let token_text = get_dlg_text(hwnd, IDC_TOKEN_EDIT as i32);

                    let mut nas = state.config.nas.clone();
                    if nas.set_addr(&text) {
                        nas.token = token_text.trim().to_string();
                        state.config.nas = nas;
                        let _ = crate::config::save(&state.config);
                        // 顶栏显示的 NAS 地址同步更新（下次重绘即生效）
                        {
                            let mut sh = state.shared.write().unwrap();
                            sh.nas_host = state.config.nas.host.clone();
                            sh.bump_seq();
                        }
                        let _ = state.cmd_tx.send(SourceCmd::SetTarget(
                            state.config.nas.addr(),
                            state.config.nas.token.clone(),
                        ));
                        // “固定在任务栏”开关
                        let chk = GetDlgItem(hwnd, IDC_TASKBAR_CHK as i32).unwrap_or_default();
                        let want_pin = SendMessageW(chk, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 != 0;
                        if want_pin != state.config.capsule.taskbar_pin {
                            state.config.capsule.taskbar_pin = want_pin;
                            let _ = crate::config::save(&state.config);
                            apply_taskbar_pin(state);
                        }
                        let _ = DestroyWindow(hwnd);
                    } else {
                        let _ = MessageBoxW(
                            hwnd,
                            pcwstr("地址格式无效。示例：192.168.1.50:8765"),
                            pcwstr("核桃胶囊 - 设置"),
                            MB_OK | MB_ICONWARNING,
                        );
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
                            if let Ok(ctl) = GetDlgItem(HWND(hwnd_isize as *mut _), IDC_TEST_RESULT as i32) {
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
                state.settings_hwnd = HWND(null_mut());
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe extern "system" fn settings_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
