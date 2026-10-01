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
use crate::ui::capsule_view::CapsuleView;
use crate::ui::View;
use std::os::raw::c_void;
use std::ptr::null_mut;
use std::sync::mpsc::channel;
use std::sync::{Arc, RwLock};
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

fn pcwstr(s: &str) -> PCWSTR {
    let v: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
    let leaked: &'static [u16] = Box::leak(v.into_boxed_slice());
    PCWSTR(leaked.as_ptr())
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

const CAPSULE_W: u32 = CapsuleView::width();
/// 初始（收起）高度，与 ui::capsule_view 内部一致。
const CAPSULE_H: u32 = 40;

/// 主入口。阻塞直到进程退出。
pub fn run(config: Config) {
    let instance = unsafe { GetModuleHandleW(None).unwrap_or_default() };

    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: instance.into(),
        lpszClassName: class_name(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        hbrBackground: HBRUSH(null_mut()),
        ..Default::default()
    };
    unsafe { let _ = RegisterClassW(&wc); }

    let shared = Arc::new(RwLock::new(SharedState::default()));
    let (cmd_tx, cmd_rx) = channel::<SourceCmd>();

    let hwnd = create_window(instance, (CAPSULE_W, CAPSULE_H));
    if hwnd.is_invalid() {
        return;
    }
    set_initial_pos(hwnd, &config);

    tray_init(hwnd);

    // 数据源
    let addr = config.nas.addr();
    let source_handle = match config.nas.source.as_str() {
        "fake" => FakeSource.spawn(SourceCtx {
            hwnd: hwnd.0 as isize,
            state: shared.clone(),
            cmd_rx,
            target: addr,
        }),
        _ => WsSource.spawn(SourceCtx {
            hwnd: hwnd.0 as isize,
            state: shared.clone(),
            cmd_rx,
            target: addr,
        }),
    };

    let state = AppState {
        hwnd,
        shared: shared.clone(),
        cmd_tx: cmd_tx.clone(),
        view: Box::new(CapsuleView::new()),
        font: Font::load(),
        pix: tiny_skia::Pixmap::new(CAPSULE_W, CAPSULE_H).unwrap(),
        back_dc: unsafe { CreateCompatibleDC(HDC(null_mut())) },
        hbmp: HBITMAP(null_mut()),
        dib_bits: null_mut(),
        dib_size: (0, 0),
        dragging: false,
        drag_start: (0, 0),
        config,
        settings_hwnd: HWND(null_mut()),
        last_seq: 0,
    };

    let raw = Box::into_raw(Box::new(state));
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize); }

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
    }
    paint_frame(unsafe { &mut *raw }, true);

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.into() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let _ = cmd_tx.send(SourceCmd::Shutdown);
    let _ = source_handle.join();
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
    config: Config,
    settings_hwnd: HWND,
    last_seq: u64,
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
            (rc.right - CAPSULE_W as i32 - 24, rc.top + 24)
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

fn tray_init(hwnd: HWND) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() },
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

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if ptr == 0 {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *(ptr as *mut AppState);

    match msg {
        WM_APP_DATA => {
            paint_frame(state, false);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = lparam.0 as i16 as i32;
            let y = (lparam.0 >> 16) as i16 as i32;
            state.dragging = true;
            state.drag_start = (x, y);
            let _ = SetCapture(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if state.dragging {
                let x = lparam.0 as i16 as i32;
                let y = (lparam.0 >> 16) as i16 as i32;
                let mut rc = RECT::default();
                let _ = GetWindowRect(hwnd, &mut rc);
                let nx = rc.left + (x - state.drag_start.0);
                let ny = rc.top + (y - state.drag_start.1);
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
                let dx = (x - state.drag_start.0).abs();
                let dy = (y - state.drag_start.1).abs();
                if dx < 4 && dy < 4 {
                    toggle_expand(state);
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
            // 设置窗口打开时它才是前台，不因此收起
            if state.view.expanded() && wparam.0 as isize != state.settings_hwnd.0 as isize {
                collapse(state);
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
            }
            LRESULT(0)
        }
        WM_DESTROY | WM_CLOSE => {
            let _ = state.cmd_tx.send(SourceCmd::Shutdown);
            tray_remove(state.hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn collapse(state: &mut AppState) {
    if !state.view.expanded() {
        return;
    }
    state.view.set_expanded(false);
    let _ = state.cmd_tx.send(SourceCmd::SetDetail(Detail::Collapsed));
    resize_window(state);
    paint_frame(state, true);
}

fn paint_frame(state: &mut AppState, force: bool) {
    let seq = state.shared.read().unwrap().seq();
    if !force && seq == state.last_seq {
        return;
    }
    state.last_seq = seq;

    let sh = state.shared.read().unwrap();
    let expanded = state.view.expanded();
    let new_h = CapsuleView::total_height(&sh, expanded);
    let new_w = CAPSULE_W;
    drop(sh);

    if state.pix.width() != new_w || state.pix.height() != new_h {
        state.pix = tiny_skia::Pixmap::new(new_w, new_h).unwrap();
        unsafe {
            if !state.hbmp.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(state.hbmp.0));
            }
        }
        state.hbmp = HBITMAP(null_mut());
        state.dib_bits = null_mut();
        state.dib_size = (0, 0);
    }

    state.pix.fill(tiny_skia::Color::TRANSPARENT);
    let sh = state.shared.read().unwrap();
    state.view.render(&mut state.pix, &sh, state.font.as_ref());
    drop(sh);

    ensure_dib(state, new_w, new_h);

    unsafe {
        let len = (new_w * new_h * 4) as usize;
        std::ptr::copy_nonoverlapping(
            state.pix.data().as_ptr(),
            state.dib_bits as *mut u8,
            len,
        );
    }

    let mut rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(state.hwnd, &mut rc);
    }
    let pt_dst = POINT { x: rc.left, y: rc.top };
    let pt_src = POINT { x: 0, y: 0 };
    let size = SIZE {
        cx: new_w as i32,
        cy: new_h as i32,
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
            state.hwnd,
            screen_dc,
            Some(&pt_dst as *const POINT),
            Some(&size as *const SIZE),
            state.back_dc,
            Some(&pt_src as *const POINT),
            COLORREF(0),
            Some(&blend as *const BLENDFUNCTION),
            ULW_ALPHA,
        );
        let _ = ReleaseDC(HWND(null_mut()), screen_dc);
    }
}

fn ensure_dib(state: &mut AppState, w: u32, h: u32) {
    if state.dib_size == (w, h) {
        return;
    }
    unsafe {
        if !state.hbmp.0.is_null() {
            let _ = DeleteObject(HGDIOBJ(state.hbmp.0));
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
    let hbmp = unsafe {
        CreateDIBSection(
            state.back_dc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            HANDLE(null_mut()),
            0,
        )
        .unwrap_or_default()
    };
    unsafe {
        let _ = SelectObject(state.back_dc, HGDIOBJ(hbmp.0));
    }
    state.hbmp = hbmp;
    state.dib_bits = bits;
    state.dib_size = (w, h);
}

fn toggle_expand(state: &mut AppState) {
    state.view.set_expanded(!state.view.expanded());
    // 订阅切换：收起只订 5 项，展开订全量
    let detail = if state.view.expanded() {
        Detail::Expanded
    } else {
        Detail::Collapsed
    };
    let _ = state.cmd_tx.send(SourceCmd::SetDetail(detail));
    if state.view.expanded() {
        unsafe {
            let _ = SetActiveWindow(state.hwnd);
        }
    }
    resize_window(state);
    paint_frame(state, true);
}

fn resize_window(state: &mut AppState) {
    let sh = state.shared.read().unwrap();
    let h = CapsuleView::total_height(&sh, state.view.expanded());
    drop(sh);
    let w = CAPSULE_W;
    unsafe {
        let _ = SetWindowPos(
            state.hwnd,
            HWND_TOPMOST,
            0, 0,
            w as i32, h as i32,
            SWP_NOMOVE | SWP_FRAMECHANGED | SWP_NOACTIVATE,
        );
    }
}

fn show_tray_menu(state: &mut AppState) {
    let menu = unsafe { CreatePopupMenu().unwrap() };
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, ID_TRAY_SETTINGS as usize, pcwstr("设置 NAS 地址"));
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

// ---- 设置窗口：NAS 地址输入 ----

const SETTINGS_CLASS: &str = "HeTaoCapsuleSettings";
const IDC_ADDR_EDIT: usize = 101;

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
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut c_void),
            ..Default::default()
        };
        unsafe { let _ = RegisterClassW(&wc); }
    });

    // 以胶囊位置为参照居中
    let mut rc = RECT::default();
    unsafe { let _ = GetWindowRect(state.hwnd, &mut rc); }
    let (w, h) = (360, 150);
    let x = rc.left + ((rc.right - rc.left) - w) / 2;
    let y = rc.top + (rc.bottom - rc.top) + 8;

    let class_wide: Vec<u16> = SETTINGS_CLASS.encode_utf16().chain(Some(0)).collect();
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_TOPMOST,
            PCWSTR(class_wide.as_ptr()),
            pcwstr("设置 NAS 地址"),
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
                mk_child("BUTTON", "确定", WINDOW_STYLE(BS_DEFPUSHBUTTON as u32), 170, 76, 80, 26, 1); // IDOK
                mk_child("BUTTON", "取消", WINDOW_STYLE(0), 258, 76, 80, 26, 2); // IDCANCEL
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = wparam.0 as u32 & 0xFFFF;
                if id == 1 {
                    // IDOK
                    let edit = GetDlgItem(hwnd, IDC_ADDR_EDIT as i32).unwrap_or_default();
                    let len = GetWindowTextLengthW(edit) as usize;
                    let mut buf = vec![0u16; len + 1];
                    GetWindowTextW(edit, &mut buf);
                    let text = String::from_utf16_lossy(&buf[..len]);
                    let mut nas = state.config.nas.clone();
                    if nas.set_addr(&text) {
                        state.config.nas = nas;
                        let _ = crate::config::save(&state.config);
                        let _ = state.cmd_tx.send(SourceCmd::SetTarget(state.config.nas.addr()));
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
