//! 核桃胶囊安装向导（自绘五步）。
//!
//! 版式依据设计交接件：520×620、纸白/墨黑/朱红、发丝分割线、五屏向导。
//! 与 `hetao-capsule-setup-core.exe`（Inno 静默核心）同目录或内嵌使用。

#![windows_subsystem = "windows"]

use std::ffi::c_void;
use std::path::PathBuf;
use std::time::Instant;

use tiny_skia::{
    Color as SkColor, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform,
};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const W: i32 = 520;
const H: i32 = 620;
const CLASS: &str = "HeTaoSetupWizard";

// ---- 设计令牌 ----
const PAPER: (u8, u8, u8) = (0xF7, 0xF5, 0xF0);
const SURFACE: (u8, u8, u8) = (0xFF, 0xFF, 0xFF);
const INK: (u8, u8, u8) = (0x15, 0x13, 0x0F);
const ON_INK: (u8, u8, u8) = (0xFF, 0xF8, 0xF4);
const VERMILION: (u8, u8, u8) = (0xC8, 0x40, 0x2B);
const MUTED: (u8, u8, u8) = (0x56, 0x52, 0x4A);
const SUBTLE: (u8, u8, u8) = (0x6B, 0x65, 0x59);
const HAIRLINE: (u8, u8, u8) = (0xDF, 0xDA, 0xCF);
const HAIRLINE_SOFT: (u8, u8, u8) = (0xEF, 0xED, 0xE7);
const BORDER: (u8, u8, u8) = (0xBD, 0xB6, 0xA8);
const SUCCESS: (u8, u8, u8) = (0x2F, 0xA3, 0x6B);
const PENDING: (u8, u8, u8) = (0xC4, 0xBE, 0xB1);

const TITLE_H: i32 = 41;
const FOOTER_H: i32 = 73;
const PAD_X: i32 = 44;
const BTN_H: i32 = 34;

const IDT_TICK: usize = 1;
const ID_DIR: i32 = 1001;

const EMBEDDED_CORE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/setup-core.bin"));

const MIT_LICENSE: &str = "MIT License

Copyright (c) 2026 蓝胖纸

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the \"Software\"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.";

#[derive(Clone, Copy, PartialEq)]
enum Screen {
    Welcome,
    Location,
    Installing,
    Done,
}

#[derive(Clone, Copy, PartialEq)]
enum Hot {
    None,
    Close,
    Min,
    Max,
    Next,
    Prev,
    Browse,
    DesktopIcon,
    Autostart,
    LaunchAfter,
    /// 完成页 / 弹层里的「MIT 许可」链接
    License,
    /// 关闭许可弹层
    LicenseClose,
}

struct Fonts {
    serif_h1: HFONT,
    serif_h2: HFONT,
    sans: HFONT,
    sans_sm: HFONT,
    sans_em: HFONT,
    mono: HFONT,
    mono_big: HFONT,
}

struct Ui {
    screen: Screen,
    hot: Hot,
    pressed: Hot,
    desktop_icon: bool,
    autostart: bool,
    launch_after: bool,
    dir: String,
    dir_edit: HWND,
    progress: f32,
    started: Option<Instant>,
    proc: HANDLE,
    /// 本次是覆盖安装（安装目录里已有客户端 exe），完成页据此换文案
    upgrade: bool,
    error: String,
    /// 许可全文弹层是否展开
    license_open: bool,
    license_scroll: i32,
    logo: Option<Pixmap>,
    fonts: Fonts,
    temp_core: Option<PathBuf>,
}

fn rgb(c: (u8, u8, u8)) -> SkColor {
    SkColor::from_rgba8(c.0, c.1, c.2, 255)
}

fn col(c: (u8, u8, u8)) -> COLORREF {
    COLORREF(((c.2 as u32) << 16) | ((c.1 as u32) << 8) | c.0 as u32)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn pcw(s: &'static str) -> PCWSTR {
    use std::sync::OnceLock;
    static MAP: OnceLock<
        std::sync::Mutex<std::collections::HashMap<&'static str, &'static [u16]>>,
    > = OnceLock::new();
    let mut g = MAP
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
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

fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

fn hit(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
}

// ---------------- 绘制基础 ----------------

unsafe fn fill(hdc: HDC, r: RECT, c: (u8, u8, u8)) {
    let b = CreateSolidBrush(col(c));
    let _ = FillRect(hdc, &r, b);
    let _ = DeleteObject(HGDIOBJ(b.0));
}

unsafe fn line(hdc: HDC, y: i32, c: (u8, u8, u8)) {
    fill(hdc, rect(0, y, W, 1), c);
}

struct TextStyle {
    font: HFONT,
    color: COLORREF,
    flags: u32,
}

unsafe fn text(hdc: HDC, s: &str, r: RECT, st: &TextStyle) {
    if !st.font.is_invalid() {
        let _ = SelectObject(hdc, HGDIOBJ(st.font.0));
    }
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, st.color);
    let mut buf = wide(s);
    let mut rc = r;
    let fmt = DRAW_TEXT_FORMAT(st.flags | DT_NOPREFIX.0);
    let _ = DrawTextW(hdc, &mut buf, &mut rc, fmt);
}

fn fnt(size: i32, weight: i32, face: &'static str) -> HFONT {
    unsafe {
        CreateFontW(
            -size,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            // ClearType 比灰度抗锯齿更实，小字号下不发虚
            CLEARTYPE_QUALITY.0 as u32,
            DEFAULT_CHARSET.0 as u32,
            pcw(face),
        )
    }
}

fn create_fonts() -> Fonts {
    // 思源宋体缺失时回退宋体；等宽缺失时回退 Consolas。
    // 字重统一抬高：12–13px 下的常规字重在浅底上笔画偏细，看久了吃力。
    Fonts {
        serif_h1: fnt(30, 700, "Source Han Serif SC"),
        serif_h2: fnt(25, 700, "Source Han Serif SC"),
        sans: fnt(13, 700, "Microsoft YaHei"),
        sans_sm: fnt(12, 700, "Microsoft YaHei"),
        sans_em: fnt(13, 700, "Microsoft YaHei"),
        mono: fnt(11, 700, "Consolas"),
        mono_big: fnt(40, 700, "Consolas"),
    }
}

fn hbitmap_from_pixmap(pix: &Pixmap) -> HBITMAP {
    unsafe {
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: pix.width() as i32,
                biHeight: -(pix.height() as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let hbm = CreateDIBSection(
            HDC::default(),
            &bi,
            DIB_RGB_COLORS,
            &mut bits,
            HANDLE(std::ptr::null_mut()),
            0,
        )
        .unwrap_or_default();
        if bits.is_null() {
            return hbm;
        }
        let src = pix.data();
        let n = (pix.width() * pix.height() * 4) as usize;
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, n);
        for i in (0..n).step_by(4) {
            dst[i] = src[i + 2];
            dst[i + 1] = src[i + 1];
            dst[i + 2] = src[i];
            dst[i + 3] = 255;
        }
        hbm
    }
}

unsafe fn draw_logo(hdc: HDC, ui: &Ui, x: i32, y: i32, size: i32) {
    if let Some(logo) = ui.logo.as_ref() {
        let hbm = hbitmap_from_pixmap(logo);
        let mem = CreateCompatibleDC(hdc);
        if !mem.is_invalid() && !hbm.is_invalid() {
            let old = SelectObject(mem, HGDIOBJ(hbm.0));
            let _ = SetStretchBltMode(hdc, HALFTONE);
            let _ = StretchBlt(
                hdc,
                x,
                y,
                size,
                size,
                mem,
                0,
                0,
                logo.width() as i32,
                logo.height() as i32,
                SRCCOPY,
            );
            let _ = SelectObject(mem, old);
        }
        let _ = DeleteDC(mem);
        let _ = DeleteObject(HGDIOBJ(hbm.0));
    }
}

// ---------------- 静态绘制 ----------------

/// 标题栏：纸底 + 图标 + 标题 + 三个窗口按钮 + 发丝线。
unsafe fn paint_title_bar(hdc: HDC, ui: &Ui) {
    fill(hdc, rect(0, 0, W, TITLE_H), PAPER);
    draw_logo(hdc, ui, 20, 12, 16);
    text(
        hdc,
        "核桃胶囊 安装程序",
        rect(44, 0, 300, TITLE_H),
        &TextStyle {
            font: ui.fonts.sans_sm,
            color: col(MUTED),
            flags: DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0,
        },
    );
    for (bx, hot) in [(488, Hot::Close), (464, Hot::Max), (440, Hot::Min)] {
        let c = if ui.hot == hot { INK } else { MUTED };
        let pen = CreatePen(PS_SOLID, 1, col(c));
        let old = SelectObject(hdc, HGDIOBJ(pen.0));
        let cx = bx + 12;
        let cy = 20;
        match hot {
            Hot::Close => {
                let _ = MoveToEx(hdc, cx - 5, cy - 5, None);
                let _ = LineTo(hdc, cx + 5, cy + 5);
                let _ = MoveToEx(hdc, cx + 5, cy - 5, None);
                let _ = LineTo(hdc, cx - 5, cy + 5);
            }
            Hot::Min => {
                let _ = MoveToEx(hdc, cx - 5, cy + 4, None);
                let _ = LineTo(hdc, cx + 5, cy + 4);
            }
            _ => {
                let _ = MoveToEx(hdc, cx - 5, cy - 4, None);
                let _ = LineTo(hdc, cx + 5, cy - 4);
                let _ = LineTo(hdc, cx + 5, cy + 5);
                let _ = LineTo(hdc, cx - 5, cy + 5);
                let _ = LineTo(hdc, cx - 5, cy - 4);
            }
        }
        let _ = SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
    line(hdc, TITLE_H - 1, HAIRLINE);
}

unsafe fn paint_footer(hdc: HDC, ui: &Ui) {
    let top = H - FOOTER_H;
    fill(hdc, rect(0, top, W, FOOTER_H), PAPER);
    line(hdc, top, HAIRLINE);

    let version = env!("CARGO_PKG_VERSION");
    let (note, secondary, primary): (String, Option<&str>, Option<&str>) = match ui.screen {
        Screen::Welcome => (format!("客户端 {version} · MIT"), None, Some("下一步")),
        Screen::Location => (
            "默认路径无需管理员权限".into(),
            Some("上一步"),
            Some("安装"),
        ),
        Screen::Installing => ("正在安装，请保持窗口开启".into(), Some("取消"), None),
        Screen::Done => (format!("客户端 {version} 已就绪"), None, Some("完成")),
    };

    text(
        hdc,
        &note,
        rect(PAD_X, top, 260, FOOTER_H),
        &TextStyle {
            font: ui.fonts.sans_sm,
            color: col(SUBTLE),
            flags: DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0,
        },
    );

    let btn_y = top + (FOOTER_H - BTN_H) / 2;
    if let Some(label) = primary {
        let r = rect(W - PAD_X - 96, btn_y, 96, BTN_H);
        let bg = if ui.hot == Hot::Next {
            (0x2B, 0x27, 0x21)
        } else {
            INK
        };
        fill(hdc, r, bg);
        text(
            hdc,
            label,
            r,
            &TextStyle {
                font: ui.fonts.sans_em,
                color: col(ON_INK),
                flags: DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0,
            },
        );
    }
    if let Some(label) = secondary {
        let r = rect(W - PAD_X - 96 - 10 - 84, btn_y, 84, BTN_H);
        fill(hdc, r, SURFACE);
        // 描边
        let hdc_pen = CreatePen(PS_SOLID, 1, col(BORDER));
        let old = SelectObject(hdc, HGDIOBJ(hdc_pen.0));
        let old_brush = SelectObject(hdc, HGDIOBJ(GetStockObject(NULL_BRUSH).0));
        let _ = Rectangle(hdc, r.left, r.top, r.right, r.bottom);
        let _ = SelectObject(hdc, old_brush);
        let _ = SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(hdc_pen.0));
        text(
            hdc,
            label,
            r,
            &TextStyle {
                font: ui.fonts.sans_em,
                color: col(INK),
                flags: DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0,
            },
        );
    }
}

unsafe fn paint_eyebrow(hdc: HDC, ui: &Ui, s: &str, color: (u8, u8, u8)) {
    text(
        hdc,
        s,
        rect(PAD_X, 70, 400, 16),
        &TextStyle {
            font: ui.fonts.mono,
            color: col(color),
            flags: DT_LEFT.0 | DT_SINGLELINE.0,
        },
    );
}

unsafe fn paint_heading(hdc: HDC, ui: &Ui, title: &str, y: i32) {
    text(
        hdc,
        title,
        rect(PAD_X, y, W - PAD_X * 2, 38),
        &TextStyle {
            font: ui.fonts.serif_h2,
            color: col(INK),
            flags: DT_LEFT.0 | DT_SINGLELINE.0,
        },
    );
}

unsafe fn paint_sub(hdc: HDC, ui: &Ui, s: &str, y: i32, h: i32) {
    text(
        hdc,
        s,
        rect(PAD_X, y, W - PAD_X * 2, h),
        &TextStyle {
            font: ui.fonts.sans,
            color: col(MUTED),
            flags: DT_LEFT.0 | DT_WORDBREAK.0,
        },
    );
}

unsafe fn paint_welcome(hdc: HDC, ui: &Ui) {
    draw_logo(hdc, ui, (W - 76) / 2, 81, 76);
    text(
        hdc,
        "核桃胶囊",
        rect(0, 171, W, 44),
        &TextStyle {
            font: ui.fonts.serif_h1,
            color: col(INK),
            flags: DT_CENTER.0 | DT_SINGLELINE.0,
        },
    );
    text(
        hdc,
        "WALNUT CAPSULE · CLIENT",
        rect(0, 216, W, 16),
        &TextStyle {
            font: ui.fonts.mono,
            color: col(VERMILION),
            flags: DT_CENTER.0 | DT_SINGLELINE.0,
        },
    );
    text(
        hdc,
        "本程序将安装核桃胶囊的 Windows 桌面客户端。它以悬浮胶囊的形式常驻桌面，随时呈现 NAS 的运行状态。",
        rect(PAD_X, 244, W - PAD_X * 2, 70),
        &TextStyle {
            font: ui.fonts.sans,
            color: col(MUTED),
            flags: DT_LEFT.0 | DT_WORDBREAK.0,
        },
    );
    line(hdc, 324, HAIRLINE_SOFT);
    let items = [
        ("01", "常驻桌面右上角，不抢焦点"),
        ("02", "纯 CPU 软渲染，不占显卡"),
        ("03", "不唤醒休眠硬盘，NAS 侧零负担"),
    ];
    for (i, (idx, s)) in items.iter().enumerate() {
        let y = 340 + i as i32 * 28;
        text(
            hdc,
            idx,
            rect(PAD_X, y, 30, 20),
            &TextStyle {
                font: ui.fonts.mono,
                color: col(VERMILION),
                flags: DT_LEFT.0 | DT_SINGLELINE.0,
            },
        );
        text(
            hdc,
            s,
            rect(PAD_X + 32, y - 1, W - PAD_X * 2 - 32, 22),
            &TextStyle {
                font: ui.fonts.sans,
                color: col(MUTED),
                flags: DT_LEFT.0 | DT_SINGLELINE.0,
            },
        );
    }
}

/// 许可全文弹层：盖在完成页之上，滚轮看全文，点空白或「关闭」收起。
unsafe fn paint_license_modal(hdc: HDC, ui: &Ui) {
    let card = rect(PAD_X, 72, W - PAD_X * 2, 470);
    fill(hdc, card, PAPER);
    let pen = CreatePen(PS_SOLID, 1, col(HAIRLINE));
    let old = SelectObject(hdc, HGDIOBJ(pen.0));
    let old_brush = SelectObject(hdc, HGDIOBJ(GetStockObject(NULL_BRUSH).0));
    let _ = Rectangle(hdc, card.left, card.top, card.right, card.bottom);
    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(pen.0));

    text(
        hdc,
        "MIT License",
        rect(PAD_X + 18, 88, 260, 24),
        &TextStyle {
            font: ui.fonts.sans_em,
            color: col(INK),
            flags: DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0,
        },
    );
    text(
        hdc,
        "Copyright (c) 2026 蓝胖纸",
        rect(PAD_X + 18, 112, 300, 18),
        &TextStyle {
            font: ui.fonts.sans_sm,
            color: col(SUBTLE),
            flags: DT_LEFT.0 | DT_SINGLELINE.0,
        },
    );
    line(hdc, 138, HAIRLINE_SOFT);

    let clip = rect(PAD_X + 18, 146, W - PAD_X * 2 - 36, 328);
    let _ = IntersectClipRect(hdc, clip.left, clip.top, clip.right, clip.bottom);
    // 标题与版权已在卡片头部画过，正文从授权段落开始，避免重复
    let body = MIT_LICENSE
        .find("Permission is hereby")
        .map(|i| &MIT_LICENSE[i..])
        .unwrap_or(MIT_LICENSE);
    text(
        hdc,
        body,
        rect(
            clip.left,
            clip.top - ui.license_scroll,
            clip.right - clip.left,
            1400,
        ),
        &TextStyle {
            font: ui.fonts.sans_sm,
            color: col(MUTED),
            flags: DT_LEFT.0 | DT_WORDBREAK.0,
        },
    );
    let _ = SelectClipRgn(hdc, None);

    let close = rect(W - PAD_X - 84, 486, 68, 28);
    fill(hdc, close, SURFACE);
    let pen = CreatePen(PS_SOLID, 1, col(BORDER));
    let old = SelectObject(hdc, HGDIOBJ(pen.0));
    let old_brush = SelectObject(hdc, HGDIOBJ(GetStockObject(NULL_BRUSH).0));
    let _ = Rectangle(hdc, close.left, close.top, close.right, close.bottom);
    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(pen.0));
    text(
        hdc,
        "关闭",
        close,
        &TextStyle {
            font: ui.fonts.sans_sm,
            color: col(INK),
            flags: DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0,
        },
    );
}

unsafe fn paint_location(hdc: HDC, ui: &Ui) {
    paint_eyebrow(hdc, ui, "STEP 02 · LOCATION", VERMILION);
    paint_heading(hdc, ui, "选择安装位置", 90);
    paint_sub(
        hdc,
        ui,
        "客户端将安装到以下目录。默认路径适用于当前用户，无需管理员权限。",
        130,
        20,
    );
    text(
        hdc,
        "安装目录",
        rect(PAD_X, 168, 120, 18),
        &TextStyle {
            font: ui.fonts.sans_em,
            color: col(INK),
            flags: DT_LEFT.0 | DT_SINGLELINE.0,
        },
    );
    // 输入框由 EDIT 子控件绘制，这里只画边框
    let edit_r = rect(PAD_X, 190, W - PAD_X * 2 - 96, 36);
    let pen = CreatePen(PS_SOLID, 1, col(BORDER));
    let old = SelectObject(hdc, HGDIOBJ(pen.0));
    let old_brush = SelectObject(hdc, HGDIOBJ(GetStockObject(NULL_BRUSH).0));
    let _ = Rectangle(hdc, edit_r.left, edit_r.top, edit_r.right, edit_r.bottom);
    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(pen.0));

    let browse_r = rect(W - PAD_X - 88, 190, 88, 36);
    fill(hdc, browse_r, SURFACE);
    let pen = CreatePen(PS_SOLID, 1, col(BORDER));
    let old = SelectObject(hdc, HGDIOBJ(pen.0));
    let old_brush = SelectObject(hdc, HGDIOBJ(GetStockObject(NULL_BRUSH).0));
    let _ = Rectangle(
        hdc,
        browse_r.left,
        browse_r.top,
        browse_r.right,
        browse_r.bottom,
    );
    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(pen.0));
    text(
        hdc,
        "浏览…",
        browse_r,
        &TextStyle {
            font: ui.fonts.sans_em,
            color: col(INK),
            flags: DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0,
        },
    );

    // 两个复选
    for (i, (label, on)) in [
        ("创建桌面快捷方式", ui.desktop_icon),
        ("开机自启动（登录后静默驻留）", ui.autostart),
    ]
    .iter()
    .enumerate()
    {
        let y = 252 + i as i32 * 30;
        let r = rect(PAD_X, y, 14, 14);
        if *on {
            draw_check_box(hdc, r.left, r.top, VERMILION);
        } else {
            fill(hdc, r, SURFACE);
            let pen = CreatePen(PS_SOLID, 1, col(BORDER));
            let old = SelectObject(hdc, HGDIOBJ(pen.0));
            let old_brush = SelectObject(hdc, HGDIOBJ(GetStockObject(NULL_BRUSH).0));
            let _ = Rectangle(hdc, r.left, r.top, r.right, r.bottom);
            let _ = SelectObject(hdc, old_brush);
            let _ = SelectObject(hdc, old);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
        text(
            hdc,
            label,
            rect(PAD_X + 24, y - 3, 380, 22),
            &TextStyle {
                font: ui.fonts.sans,
                color: col(MUTED),
                flags: DT_LEFT.0 | DT_SINGLELINE.0,
            },
        );
    }
}

unsafe fn paint_installing(hdc: HDC, ui: &Ui) {
    paint_eyebrow(hdc, ui, "STEP 03 · INSTALLING", VERMILION);
    paint_heading(hdc, ui, "正在安装", 90);
    text(
        hdc,
        &format!("{:.0}%", ui.progress * 100.0),
        rect(PAD_X, 126, 170, 52),
        &TextStyle {
            font: ui.fonts.mono_big,
            color: col(INK),
            flags: DT_LEFT.0 | DT_SINGLELINE.0,
        },
    );
    text(
        hdc,
        "正在解压客户端文件",
        rect(PAD_X + 178, 145, 260, 20),
        &TextStyle {
            font: ui.fonts.sans,
            color: col(MUTED),
            flags: DT_LEFT.0 | DT_SINGLELINE.0,
        },
    );
    let track = rect(PAD_X, 186, W - PAD_X * 2, 6);
    fill(hdc, track, HAIRLINE_SOFT);
    let fill_w = ((W - PAD_X * 2) as f32 * ui.progress).max(0.0) as i32;
    if fill_w > 0 {
        fill(hdc, rect(PAD_X, 186, fill_w, 6), VERMILION);
    }
    text(
        hdc,
        "安装过程中请勿关闭此窗口。客户端会在完成后自动启动。",
        rect(PAD_X, 206, W - PAD_X * 2, 20),
        &TextStyle {
            font: ui.fonts.sans_sm,
            color: col(SUBTLE),
            flags: DT_LEFT.0 | DT_SINGLELINE.0,
        },
    );
    line(hdc, 238, HAIRLINE_SOFT);

    let steps = [
        ("创建安装目录", 0.15),
        ("解压客户端可执行文件", 0.45),
        ("写入注册表与快捷方式", 0.80),
        ("配置开机自启", 0.99),
    ];
    for (i, (label, threshold)) in steps.iter().enumerate() {
        let y = 254 + i as i32 * 34;
        let done = ui.progress >= *threshold;
        let active = !done && ui.progress >= threshold - 0.30;
        if done {
            draw_check_box(hdc, PAD_X, y, SUCCESS);
        } else if active {
            fill(hdc, rect(PAD_X + 2, y + 2, 10, 10), VERMILION);
        } else {
            let pen = CreatePen(PS_SOLID, 1, col(PENDING));
            let old = SelectObject(hdc, HGDIOBJ(pen.0));
            let old_brush = SelectObject(hdc, HGDIOBJ(GetStockObject(NULL_BRUSH).0));
            let _ = Ellipse(hdc, PAD_X, y, PAD_X + 14, y + 14);
            let _ = SelectObject(hdc, old_brush);
            let _ = SelectObject(hdc, old);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
        text(
            hdc,
            label,
            rect(PAD_X + 28, y - 3, 380, 22),
            &TextStyle {
                font: if active {
                    ui.fonts.sans_em
                } else {
                    ui.fonts.sans
                },
                color: col(if done {
                    SUBTLE
                } else if active {
                    INK
                } else {
                    PENDING
                }),
                flags: DT_LEFT.0 | DT_SINGLELINE.0,
            },
        );
    }
    if !ui.error.is_empty() {
        text(
            hdc,
            &ui.error,
            rect(PAD_X, 410, W - PAD_X * 2, 40),
            &TextStyle {
                font: ui.fonts.sans_sm,
                color: col(VERMILION),
                flags: DT_LEFT.0 | DT_WORDBREAK.0,
            },
        );
    }
}

/// 把离屏 Pixmap 按原尺寸贴到窗口 DC。
unsafe fn blit_pixmap(hdc: HDC, pix: &Pixmap, x: i32, y: i32) {
    let hbm = hbitmap_from_pixmap(pix);
    let mem = CreateCompatibleDC(hdc);
    if !mem.is_invalid() && !hbm.is_invalid() {
        let old = SelectObject(mem, HGDIOBJ(hbm.0));
        let _ = StretchBlt(
            hdc,
            x,
            y,
            pix.width() as i32,
            pix.height() as i32,
            mem,
            0,
            0,
            pix.width() as i32,
            pix.height() as i32,
            SRCCOPY,
        );
        let _ = SelectObject(mem, old);
    }
    let _ = DeleteDC(mem);
    let _ = DeleteObject(HGDIOBJ(hbm.0));
}

/// 14px 勾选块：实心方 + 抗锯齿白勾。
///
/// 字体里的 ✓ 在 12px 下会糊成一团，这里整块离屏绘制后一次贴上屏。
unsafe fn draw_check_box(hdc: HDC, x: i32, y: i32, bg: (u8, u8, u8)) {
    let s = 14u32;
    let Some(mut pix) = Pixmap::new(s, s) else {
        return;
    };
    pix.fill(rgb(bg));

    let mut paint = Paint::default();
    paint.set_color(rgb(ON_INK));
    paint.anti_alias = true;
    let mut pb = PathBuilder::new();
    pb.move_to(3.4, 7.2);
    pb.line_to(6.1, 10.0);
    pb.line_to(10.7, 4.6);
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: 2.0,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        pix.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
    blit_pixmap(hdc, &pix, x, y);
}

/// 完成徽标：绿圆 + 对勾整块用 tiny-skia 抗锯齿绘制，再贴回窗口 DC。
///
/// GDI 的 Ellipse / Polyline 都不做抗锯齿，56px 的圆边缘会是明显阶梯；
/// 这里离屏画到 Pixmap（底色取内容区白），一次 StretchBlt 上屏。
unsafe fn draw_badge(hdc: HDC, cx: i32, cy: i32, r: i32) {
    let size = (r * 2 + 2) as u32;
    let Some(mut pix) = Pixmap::new(size, size) else {
        return;
    };
    pix.fill(rgb(SURFACE));
    let lc = (r + 1) as f32;

    let mut disc = Paint::default();
    disc.set_color(rgb(SUCCESS));
    disc.anti_alias = true;
    let mut pb = PathBuilder::new();
    pb.push_circle(lc, lc, r as f32 - 0.5);
    if let Some(path) = pb.finish() {
        pix.fill_path(&path, &disc, FillRule::Winding, Transform::identity(), None);
    }

    let mut mark = Paint::default();
    mark.set_color(rgb(ON_INK));
    mark.anti_alias = true;
    let mut pb = PathBuilder::new();
    pb.move_to(lc - 11.0, lc + 1.0);
    pb.line_to(lc - 3.0, lc + 9.0);
    pb.line_to(lc + 12.0, lc - 8.0);
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: 5.0,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        pix.stroke_path(&path, &mark, &stroke, Transform::identity(), None);
    }

    blit_pixmap(hdc, &pix, cx - r - 1, cy - r - 1);
}

unsafe fn paint_done(hdc: HDC, ui: &Ui) {
    paint_eyebrow(hdc, ui, "STEP 04 · COMPLETE", SUCCESS);
    // 完成徽标：56px 绿圆 + 对勾
    draw_badge(hdc, W / 2, 120, 28);
    text(
        hdc,
        "安装完成",
        rect(0, 162, W, 40),
        &TextStyle {
            font: ui.fonts.serif_h2,
            color: col(INK),
            flags: DT_CENTER.0 | DT_SINGLELINE.0,
        },
    );
    let (tip, launch_label) = if ui.upgrade {
        // 覆盖安装：配置与主题都在，不引导、不提设置页
        (
            "客户端已更新完成，原有 NAS 配置、主题与显示设置保持不变。",
            "立即启动核桃胶囊",
        )
    } else {
        (
            "客户端已安装。点击完成后会打开设置页，填入 NAS 地址与配对令牌即可开始使用。",
            "完成后打开设置页",
        )
    };
    text(
        hdc,
        tip,
        rect(PAD_X, 206, W - PAD_X * 2, 42),
        &TextStyle {
            font: ui.fonts.sans,
            color: col(MUTED),
            flags: DT_LEFT.0 | DT_WORDBREAK.0,
        },
    );
    // 立即启动
    let y = 276;
    let r = rect(PAD_X, y, 14, 14);
    if ui.launch_after {
        draw_check_box(hdc, r.left, r.top, VERMILION);
    } else {
        fill(hdc, r, SURFACE);
        let pen = CreatePen(PS_SOLID, 1, col(BORDER));
        let old = SelectObject(hdc, HGDIOBJ(pen.0));
        let old_brush = SelectObject(hdc, HGDIOBJ(GetStockObject(NULL_BRUSH).0));
        let _ = Rectangle(hdc, r.left, r.top, r.right, r.bottom);
        let _ = SelectObject(hdc, old_brush);
        let _ = SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
    text(
        hdc,
        launch_label,
        rect(PAD_X + 24, y - 3, 300, 22),
        &TextStyle {
            font: ui.fonts.sans,
            color: col(MUTED),
            flags: DT_LEFT.0 | DT_SINGLELINE.0,
        },
    );
    // 许可全文入口：安装目录里也有 LICENSE.txt
    text(
        hdc,
        "MIT 许可",
        rect(W - PAD_X - 96, y - 3, 96, 22),
        &TextStyle {
            font: ui.fonts.sans_sm,
            color: col(VERMILION),
            flags: DT_RIGHT.0 | DT_SINGLELINE.0,
        },
    );
}

unsafe fn paint(hdc: HDC, ui: &Ui) {
    fill(
        hdc,
        rect(0, 0, W, H),
        if ui.screen == Screen::Welcome {
            PAPER
        } else {
            SURFACE
        },
    );
    paint_title_bar(hdc, ui);
    match ui.screen {
        Screen::Welcome => paint_welcome(hdc, ui),
        Screen::Location => paint_location(hdc, ui),
        Screen::Installing => paint_installing(hdc, ui),
        Screen::Done => paint_done(hdc, ui),
    }
    paint_footer(hdc, ui);
    if ui.license_open {
        paint_license_modal(hdc, ui);
    }
}

// ---------------- 交互与流程 ----------------

fn sibling(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(name))
}

fn resolve_core() -> Option<(PathBuf, bool)> {
    if !EMBEDDED_CORE.is_empty() {
        let path =
            std::env::temp_dir().join(format!("hetao-capsule-setup-{}.exe", std::process::id()));
        if std::fs::write(&path, EMBEDDED_CORE).is_ok() {
            return Some((path, true));
        }
    }
    let path = sibling("hetao-capsule-setup-core.exe")?;
    path.exists().then_some((path, false))
}

fn default_dir() -> String {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:/Users"));
    local
        .join("Programs")
        .join("核桃胶囊")
        .display()
        .to_string()
}

/// 本机是否已经配对过 NAS（配置文件存在且 token 非空）。
///
/// 只做布尔判断，不读取、不展示任何凭据内容。
fn config_paired() -> bool {
    let path = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("hetao-capsule").join("config.toml"));
    let Some(path) = path else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let mut in_nas = false;
    for raw in text.lines() {
        let l = raw.trim();
        if l.starts_with('[') {
            in_nas = l == "[nas]";
            continue;
        }
        if !in_nas {
            continue;
        }
        if let Some(v) = l.strip_prefix("token") {
            return !v
                .trim_start_matches([' ', '='])
                .trim_matches('"')
                .is_empty();
        }
    }
    false
}

fn start_process(exe: &PathBuf, args: &str) -> std::io::Result<HANDLE> {
    let mut cmd: Vec<u16> = if args.is_empty() {
        format!("\"{}\"", exe.display())
    } else {
        format!("\"{}\" {args}", exe.display())
    }
    .encode_utf16()
    .collect();
    cmd.push(0);
    let si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut pi = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            None,
            PWSTR(cmd.as_mut_ptr()),
            None,
            None,
            false,
            CREATE_NO_WINDOW,
            None,
            None,
            &si,
            &mut pi,
        )
        .map_err(|e| std::io::Error::other(e.to_string()))?;
        let _ = CloseHandle(pi.hThread);
        Ok(pi.hProcess)
    }
}

unsafe fn set_dir_visible(ui: &Ui, visible: bool) {
    if !ui.dir_edit.is_invalid() {
        let _ = ShowWindow(ui.dir_edit, if visible { SW_SHOW } else { SW_HIDE });
    }
}

unsafe fn begin_install(ui: &mut Ui, hwnd: HWND) {
    let Some((core, temp)) = resolve_core() else {
        ui.error = "找不到安装核心文件，请重新下载安装程序".into();
        return;
    };
    // 覆盖安装的判据要与客户端"是否弹设置页"一致：
    // 目录里已有 exe，或本机已配对过 NAS，都不再引导去设置页
    ui.upgrade = PathBuf::from(&ui.dir).join("hetao-capsule.exe").exists() || config_paired();
    if temp {
        ui.temp_core = Some(core.clone());
    }
    let mut tasks: Vec<&str> = Vec::new();
    if ui.desktop_icon {
        tasks.push("desktopicon");
    }
    if ui.autostart {
        tasks.push("startup");
    }
    let args = format!(
        "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /DIR=\"{}\" /TASKS=\"{}\"",
        ui.dir,
        tasks.join(",")
    );
    match start_process(&core, &args) {
        Ok(handle) => {
            ui.proc = handle;
            ui.screen = Screen::Installing;
            ui.progress = 0.02;
            ui.started = Some(Instant::now());
            ui.error.clear();
            set_dir_visible(ui, false);
            let _ = SetTimer(hwnd, IDT_TICK, 120, None);
        }
        Err(e) => {
            ui.error = format!("无法启动安装程序：{e}");
        }
    }
}

unsafe fn goto(ui: &mut Ui, screen: Screen) {
    ui.screen = screen;
    set_dir_visible(ui, screen == Screen::Location);
}

unsafe fn tick(ui: &mut Ui, hwnd: HWND) {
    if ui.screen != Screen::Installing {
        return;
    }
    if let Some(t0) = ui.started {
        let secs = t0.elapsed().as_secs_f32();
        // 静默安装通常几秒；按时间推进到 95%，进程退出后再补到 100%
        let target = (secs / 4.0).min(1.0) * 0.95;
        if target > ui.progress {
            ui.progress = target;
        }
    }
    if !ui.proc.is_invalid() && WaitForSingleObject(ui.proc, 0) == WAIT_OBJECT_0 {
        let mut code = 0u32;
        let _ = GetExitCodeProcess(ui.proc, &mut code);
        let _ = CloseHandle(ui.proc);
        ui.proc = HANDLE::default();
        let _ = KillTimer(hwnd, IDT_TICK);
        if code == 0 {
            ui.progress = 1.0;
            // 客户端只在点「完成」时启动：安装完就自动拉起会让首次运行的设置页
            // 抢在完成页前面弹出来
            goto(ui, Screen::Done);
        } else {
            ui.error = format!("安装失败（代码 {code}），请重试或查看临时目录日志");
        }
        cleanup_temp(ui);
    }
    let _ = InvalidateRect(hwnd, None, TRUE);
}

fn cleanup_temp(ui: &mut Ui) {
    if let Some(p) = ui.temp_core.take() {
        let _ = std::fs::remove_file(p);
    }
}

unsafe fn on_click(ui: &mut Ui, hwnd: HWND, hit_hot: Hot) {
    match hit_hot {
        Hot::Close => {
            let _ = DestroyWindow(hwnd);
        }
        Hot::Min => {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        }
        Hot::Max => {}
        Hot::Next => match ui.screen {
            Screen::Welcome => goto(ui, Screen::Location),
            Screen::Location => begin_install(ui, hwnd),
            Screen::Installing => {}
            Screen::Done => {
                if ui.launch_after {
                    let exe = PathBuf::from(&ui.dir).join("hetao-capsule.exe");
                    if exe.exists() {
                        let _ = start_process(&exe, "");
                    }
                }
                let _ = DestroyWindow(hwnd);
            }
        },
        Hot::Prev => match ui.screen {
            Screen::Location => goto(ui, Screen::Welcome),
            Screen::Installing => {
                if !ui.proc.is_invalid() {
                    let _ = TerminateProcess(ui.proc, 1);
                    let _ = CloseHandle(ui.proc);
                    ui.proc = HANDLE::default();
                }
                let _ = KillTimer(hwnd, IDT_TICK);
                ui.progress = 0.0;
                ui.started = None;
                ui.error.clear();
                goto(ui, Screen::Location);
                let _ = MessageBoxW(
                    hwnd,
                    PCWSTR(wide("已取消安装。可能残留部分文件，可重新安装覆盖。").as_ptr()),
                    PCWSTR(wide("核桃胶囊 安装程序").as_ptr()),
                    MB_OK | MB_ICONWARNING,
                );
            }
            _ => {}
        },
        Hot::Browse => {
            if let Some(dir) = pick_folder(hwnd) {
                ui.dir = dir;
                let mut w: Vec<u16> = ui.dir.encode_utf16().chain(Some(0)).collect();
                let _ = SetWindowTextW(ui.dir_edit, PCWSTR(w.as_mut_ptr()));
            }
        }
        Hot::License => {
            ui.license_open = true;
            ui.license_scroll = 0;
            let _ = InvalidateRect(hwnd, None, TRUE);
        }
        Hot::LicenseClose => {
            ui.license_open = false;
            let _ = InvalidateRect(hwnd, None, TRUE);
        }
        Hot::DesktopIcon => {
            ui.desktop_icon = !ui.desktop_icon;
            let _ = InvalidateRect(hwnd, None, TRUE);
        }
        Hot::Autostart => {
            ui.autostart = !ui.autostart;
            let _ = InvalidateRect(hwnd, None, TRUE);
        }
        Hot::LaunchAfter => {
            ui.launch_after = !ui.launch_after;
            let _ = InvalidateRect(hwnd, None, TRUE);
        }
        Hot::None => {}
    }
}

unsafe fn pick_folder(hwnd: HWND) -> Option<String> {
    let mut display = wide("选择安装目录");
    let mut title = wide("选择安装目录");
    let bi = BROWSEINFOW {
        hwndOwner: hwnd,
        pszDisplayName: PWSTR(display.as_mut_ptr()),
        lpszTitle: PCWSTR(title.as_mut_ptr()),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        ..Default::default()
    };
    let pidl = SHBrowseForFolderW(&bi);
    if pidl.is_null() {
        return None;
    }
    let mut buf = [0u16; 260];
    let ok = SHGetPathFromIDListW(pidl, &mut buf).as_bool();
    let _ = CoTaskMemFree(Some(pidl as *const c_void));
    if ok {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&buf[..len]))
    } else {
        None
    }
}

fn hot_at(ui: &Ui, x: i32, y: i32) -> Hot {
    // 许可弹层展开时吃掉所有点击：点「关闭」或卡片外的空白都收起
    if ui.license_open {
        let card = rect(PAD_X, 72, W - PAD_X * 2, 470);
        let close = rect(W - PAD_X - 84, 486, 68, 28);
        if hit(close, x, y) || !hit(card, x, y) {
            return Hot::LicenseClose;
        }
        return Hot::None;
    }
    if hit(rect(488, 8, 24, 24), x, y) {
        return Hot::Close;
    }
    if hit(rect(464, 8, 24, 24), x, y) {
        return Hot::Max;
    }
    if hit(rect(440, 8, 24, 24), x, y) {
        return Hot::Min;
    }
    let btn_y = H - FOOTER_H + (FOOTER_H - BTN_H) / 2;
    let primary = matches!(ui.screen, Screen::Welcome | Screen::Location | Screen::Done);
    if primary && hit(rect(W - PAD_X - 96, btn_y, 96, BTN_H), x, y) {
        return Hot::Next;
    }
    if matches!(ui.screen, Screen::Location | Screen::Installing)
        && hit(rect(W - PAD_X - 96 - 10 - 84, btn_y, 84, BTN_H), x, y)
    {
        return Hot::Prev;
    }
    match ui.screen {
        Screen::Location => {
            if hit(rect(W - PAD_X - 88, 190, 88, 36), x, y) {
                Hot::Browse
            } else if hit(rect(PAD_X, 246, W - PAD_X * 2, 28), x, y) {
                Hot::DesktopIcon
            } else if hit(rect(PAD_X, 276, W - PAD_X * 2, 28), x, y) {
                Hot::Autostart
            } else {
                Hot::None
            }
        }
        Screen::Done => {
            // 「MIT 许可」优先判定：它就在启动勾选行的右端，宽泛的行热区会把它吃掉
            if hit(rect(W - PAD_X - 104, 266, 108, 34), x, y) {
                Hot::License
            } else if hit(rect(PAD_X, 270, 300, 28), x, y) {
                Hot::LaunchAfter
            } else {
                Hot::None
            }
        }
        _ => Hot::None,
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if ptr == 0 {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let ui = &mut *(ptr as *mut Ui);
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint(hdc, ui);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_NCHITTEST => {
            // 标题栏区域可拖动窗口
            let mut pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            if pt.y < TITLE_H && !hit(rect(432, 0, 88, TITLE_H), pt.x, pt.y) && pt.x < W - 4 {
                return LRESULT(HTCAPTION as isize);
            }
            LRESULT(HTCLIENT as isize)
        }
        WM_TIMER if wparam.0 == IDT_TICK => {
            tick(ui, hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let hot = hot_at(ui, x, y);
            if hot != ui.hot {
                ui.hot = hot;
                let _ = InvalidateRect(hwnd, None, TRUE);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            if ui.license_open {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
                ui.license_scroll = (ui.license_scroll - delta / 3).clamp(0, 760);
                let _ = InvalidateRect(hwnd, None, TRUE);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            ui.pressed = hot_at(
                ui,
                (lparam.0 & 0xFFFF) as i16 as i32,
                ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            );
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let hot = hot_at(ui, x, y);
            ui.pressed = Hot::None;
            if hot != Hot::None {
                on_click(ui, hwnd, hot);
            }
            // 切屏 / 勾选状态变化后必须整窗重绘，否则会停留在上一屏画面
            let _ = InvalidateRect(hwnd, None, TRUE);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(hwnd, IDT_TICK);
            if !ui.proc.is_invalid() {
                let _ = TerminateProcess(ui.proc, 1);
                let _ = CloseHandle(ui.proc);
            }
            cleanup_temp(ui);
            for f in [
                ui.fonts.serif_h1,
                ui.fonts.serif_h2,
                ui.fonts.sans,
                ui.fonts.sans_sm,
                ui.fonts.sans_em,
                ui.fonts.mono,
                ui.fonts.mono_big,
            ] {
                if !f.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(f.0));
                }
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    // 清理上次异常退出残留的临时安装核心（同前缀即认作本程序释放的文件）
    let tmp = std::env::temp_dir();
    if let Ok(rd) = std::fs::read_dir(&tmp) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("hetao-capsule-setup-") && name.ends_with(".exe") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: pcw(CLASS),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: CreateSolidBrush(col(PAPER)),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            pcw(CLASS),
            pcw("核桃胶囊 安装程序"),
            WS_POPUP,
            (sw - W) / 2,
            (sh - H) / 2,
            W,
            H,
            None,
            None,
            hinst,
            None,
        )
        .unwrap_or_default();
        if hwnd.0.is_null() {
            return;
        }
        let region = CreateRoundRectRgn(0, 0, W + 1, H + 1, 18, 18);
        let _ = SetWindowRgn(hwnd, region, TRUE);

        let logo_bytes = include_bytes!("../../assets/setup-logo.png").to_vec();
        // 预合成到纸色底：StretchBlt 不做 alpha 混合，透明边会变成黑块
        let logo = Pixmap::decode_png(&logo_bytes).ok().and_then(|src| {
            let mut base = Pixmap::new(src.width(), src.height())?;
            base.fill(rgb(PAPER));
            base.draw_pixmap(
                0,
                0,
                src.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                tiny_skia::Transform::identity(),
                None,
            );
            Some(base)
        });

        let dir = default_dir();
        let mut w: Vec<u16> = dir.encode_utf16().chain(Some(0)).collect();
        let dir_edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            pcw("EDIT"),
            PCWSTR(w.as_mut_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | ES_AUTOHSCROLL as u32),
            PAD_X + 6,
            196,
            W - PAD_X * 2 - 96 - 12,
            24,
            hwnd,
            HMENU(ID_DIR as *mut c_void),
            hinst,
            None,
        )
        .unwrap_or_default();
        let edit_font = fnt(12, 400, "Microsoft YaHei");
        let _ = SendMessageW(
            dir_edit,
            WM_SETFONT,
            WPARAM(edit_font.0 as usize),
            LPARAM(1),
        );
        let _ = ShowWindow(dir_edit, SW_HIDE);

        let ui = Box::new(Ui {
            screen: Screen::Welcome,
            hot: Hot::None,
            pressed: Hot::None,
            desktop_icon: false,
            autostart: true,
            launch_after: true,
            dir,
            dir_edit,
            progress: 0.0,
            started: None,
            proc: HANDLE::default(),
            upgrade: false,
            error: String::new(),
            license_open: false,
            license_scroll: 0,
            logo,
            fonts: create_fonts(),
            temp_core: None,
        });
        let raw = Box::into_raw(ui);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = Box::from_raw(raw);
    }
}
