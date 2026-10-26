//! 字体光栅化与文本绘制。
//!
//! 字形由 **GDI** 光栅化（`GetGlyphOutlineW` + `GGO_GRAY8_BITMAP`）：
//! 走系统光栅化器、带网格对齐（hinting），小字号汉字的横竖笔画会落到像素上，
//! 而不是像纯轮廓光栅化那样把覆盖率劈到相邻两行、整行发虚。
//!
//! 位图按 (字符, 字号, 粗体) 缓存：界面字符集很小且每帧重复，
//! 首次出现的字符走一次 GDI 光栅化，之后只做内存 blit。
//!
//! 注意：GDI 只支持**整数**字号（设备像素），所以绘制尺寸按 `round` 取整——
//! 这与系统原生文字一致的约束（Windows 自身也用整数字号）。
//! 灰度抗锯齿用 `ANTIALIASED_QUALITY`：分层窗口逐像素 alpha，不能上 ClearType
//! （ClearType 需要不透明底才能做子像素混合）。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use tiny_skia::{Color, Pixmap};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateFontIndirectW, GetGlyphOutlineW, GetTextExtentPoint32W,
    GetTextMetricsW, SelectObject, ANTIALIASED_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DEFAULT_PITCH, FF_DONTCARE, FIXED, FW_BOLD, FW_NORMAL, GGO_GRAY8_BITMAP, GLYPHMETRICS, HDC,
    HFONT, HGDIOBJ, LOGFONTW, MAT2, OUT_TT_PRECIS, TEXTMETRICW,
};

/// 界面字体：微软雅黑（粗体由 GDI 按 `FW_BOLD` 自动选 msyhbd）。
const FACE: &str = "Microsoft YaHei";

/// 字形缓存键：同一字符同一字号同一粗细的位图与位置无关，可全局复用。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    /// Unicode 码点
    ch: u32,
    size_bits: u32,
    bold: bool,
}

/// 以笔尖（pen position，即 baseline 起点）为原点的字形 alpha 位图。
struct GlyphBmp {
    w: usize,
    h: usize,
    /// 位图左上角相对笔尖 x 的偏移（通常为左侧轴承）。
    left: f32,
    /// 位图左上角相对笔尖 y 的偏移（基线以上为负）。
    top: f32,
    advance: f32,
    /// w*h 覆盖率 alpha，0=透明。
    cov: Vec<u8>,
}

impl GlyphBmp {
    fn empty(advance: f32) -> Self {
        Self {
            w: 0,
            h: 0,
            left: 0.0,
            top: 0.0,
            advance,
            cov: Vec::new(),
        }
    }
}

pub struct Font {
    /// 独占的内存 DC（字形光栅化与度量都走它，字体按需选入）
    dc: HDC,
    /// DC 当前实际选入的字体句柄。DC 的"当前字体"是隐式全局状态，
    /// 必须跟踪它，否则命中缓存的字体不会被重新选入，出图用的是上一个字体。
    cur: Cell<*mut c_void>,
    /// (整数像素字号, 粗体) -> HFONT
    fonts: RefCell<HashMap<(u32, bool), HFONT>>,
    /// 整数像素字号 -> ascent
    ascents: RefCell<HashMap<u32, f32>>,
    cache: RefCell<HashMap<GlyphKey, GlyphBmp>>,
}

impl Font {
    pub fn load() -> Option<Font> {
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            return None;
        }
        Some(Font {
            dc,
            cur: Cell::new(std::ptr::null_mut()),
            fonts: RefCell::new(HashMap::new()),
            ascents: RefCell::new(HashMap::new()),
            cache: RefCell::new(HashMap::new()),
        })
    }

    /// 取指定字号/粗细的字体句柄，并**确保它已选入 DC**。
    /// GDI 字号是整数，小数尺寸在这里统一按四舍五入收敛。
    ///
    /// 必须每次调用都 `SelectObject`：字体是缓存的句柄，但 DC 的"当前字体"是全局状态。
    /// 命中缓存却跳过选入的话，DC 里还停在上一个字号/字重的字体上，`GetGlyphOutlineW`
    /// 就会按错字体出图，而且这张错图会被字形缓存永久记住——表现为同一个字串里
    /// 个别字比别人大/粗（哪个字先被光栅化、当时 DC 恰好是什么字体，纯靠运气）。
    fn select(&self, size: f32, bold: bool) -> HFONT {
        let h = size.round().max(1.0) as i32;
        let key = (h as u32, bold);
        // 先用 copied() 把值取出来、让借用立即结束——不能写成
        // `match self.fonts.borrow().get(&key) {...}`：那样 Ref 临时值会活到整个 let 语句结束，
        // None 分支里的 borrow_mut() 就会撞上 already-borrowed panic（panic=abort → 闪退）。
        let cached = self.fonts.borrow().get(&key).copied();
        let hf = match cached {
            Some(f) => f,
            None => {
                let mut lf = LOGFONTW::default();
                lf.lfHeight = -h; // 负值 = 字符高度（不含内部行距）
                lf.lfWeight = if bold {
                    FW_BOLD.0 as i32
                } else {
                    FW_NORMAL.0 as i32
                };
                lf.lfCharSet = DEFAULT_CHARSET;
                lf.lfOutPrecision = OUT_TT_PRECIS;
                lf.lfClipPrecision = CLIP_DEFAULT_PRECIS;
                lf.lfQuality = ANTIALIASED_QUALITY;
                lf.lfPitchAndFamily = DEFAULT_PITCH.0 | FF_DONTCARE.0;
                let face: Vec<u16> = FACE.encode_utf16().collect();
                lf.lfFaceName[..face.len()].copy_from_slice(&face);
                let created = unsafe { CreateFontIndirectW(&lf) };
                self.fonts.borrow_mut().insert(key, created);
                created
            }
        };
        if self.cur.get() != hf.0 {
            unsafe {
                SelectObject(self.dc, HGDIOBJ(hf.0));
            }
            self.cur.set(hf.0);
        }
        hf
    }

    pub fn ascent(&self, size: f32) -> f32 {
        let key = size.round().max(1.0) as u32;
        if let Some(a) = self.ascents.borrow().get(&key) {
            return *a;
        }
        self.select(size, false);
        let mut tm = TEXTMETRICW::default();
        let ok = unsafe { GetTextMetricsW(self.dc, &mut tm) };
        let v = if ok.as_bool() {
            tm.tmAscent as f32
        } else {
            size * 0.76
        };
        self.ascents.borrow_mut().insert(key, v);
        v
    }

    /// 预热界面静态用字的字形缓存：在启动空闲期把每个字号下的常用字符
    /// 全部光栅化一遍，避免展开首帧/首份面板构建时同步跑几十个字形的冷光栅化。
    /// 粗细两套都预热；字号列表覆盖界面实际用到的整数高度。
    pub fn preheat(&self) {
        // 界面静态字符串用到的汉字 + 数字/单位/标点 + 全字母（GPU 名/主机名多为 ASCII）
        const CHARS: &str = concat!(
            "使用率温度内存显存存储状态块盘休眠中运行未知已用容量硬盘读写收发连接后重等待数据正在断开",
            "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
            " .%°…·/-,",
        );
        const SIZES: [f32; 10] = [7.5, 8.5, 10.0, 10.5, 11.0, 11.5, 12.5, 13.0, 13.5, 16.0];
        for &size in &SIZES {
            for c in CHARS.chars() {
                self.ensure(size, false, c as u32);
            }
            for c in CHARS.chars() {
                self.ensure(size, true, c as u32);
            }
        }
    }

    /// 文本宽度（用于右对齐与截断）。
    pub fn measure(&self, size: f32, text: &str) -> f32 {
        self.measure_impl(size, text, false)
    }

    /// 粗体文本宽度（粗体字宽略宽于常规，截断需按粗体测量）。
    pub fn measure_bold(&self, size: f32, text: &str) -> f32 {
        self.measure_impl(size, text, true)
    }

    fn measure_impl(&self, size: f32, text: &str, bold: bool) -> f32 {
        let mut w = 0.0;
        for c in text.chars() {
            let ch = c as u32;
            self.ensure(size, bold, ch);
            let key = GlyphKey {
                ch,
                size_bits: size.to_bits(),
                bold,
            };
            if let Some(b) = self.cache.borrow().get(&key) {
                w += b.advance;
            }
        }
        w
    }

    /// 在 (x, baseline) 处绘制文本，返回末端的 x。
    pub fn draw(
        &self,
        px: &mut Pixmap,
        x: f32,
        baseline: f32,
        size: f32,
        text: &str,
        color: Color,
    ) -> f32 {
        self.draw_impl(px, x, baseline, size, text, color, false)
    }

    /// 粗体文本绘制（无粗体字体时回退常规字形）。
    pub fn draw_bold(
        &self,
        px: &mut Pixmap,
        x: f32,
        baseline: f32,
        size: f32,
        text: &str,
        color: Color,
    ) -> f32 {
        self.draw_impl(px, x, baseline, size, text, color, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_impl(
        &self,
        px: &mut Pixmap,
        x: f32,
        baseline: f32,
        size: f32,
        text: &str,
        color: Color,
        bold: bool,
    ) -> f32 {
        let mut caret = x;
        for c in text.chars() {
            let ch = c as u32;
            self.ensure(size, bold, ch);
            let key = GlyphKey {
                ch,
                size_bits: size.to_bits(),
                bold,
            };
            let cache = self.cache.borrow();
            let Some(bmp) = cache.get(&key) else { continue };
            blit_glyph(px, caret, baseline, bmp, color);
            caret += bmp.advance;
        }
        caret
    }

    /// 缓存缺失时补一次 GDI 光栅化。
    fn ensure(&self, size: f32, bold: bool, ch: u32) {
        let key = GlyphKey {
            ch,
            size_bits: size.to_bits(),
            bold,
        };
        if self.cache.borrow().contains_key(&key) {
            return;
        }
        let bmp = self.rasterize(size, bold, ch);
        self.cache.borrow_mut().insert(key, bmp);
    }

    /// 用 GDI 取一个字形的灰度覆盖率位图。
    fn rasterize(&self, size: f32, bold: bool, ch: u32) -> GlyphBmp {
        self.select(size, bold);
        // MAT2 单位矩阵（FIXED 是 16.16：整数部分在 value）
        let one = FIXED { fract: 0, value: 1 };
        let zero = FIXED { fract: 0, value: 0 };
        let mat = MAT2 {
            eM11: one,
            eM12: zero,
            eM21: zero,
            eM22: one,
        };

        // 第一次调用只取度量（cbBuffer=0）
        let mut gm = GLYPHMETRICS::default();
        let n = unsafe { GetGlyphOutlineW(self.dc, ch, GGO_GRAY8_BITMAP, &mut gm, 0, None, &mat) };
        let advance = gm.gmCellIncX as f32;
        if n == 0 || n == u32::MAX {
            // 0 = 无位图（空格）；GDI_ERROR = 该字体没有这个字形 → 用 extent 兜一个推进宽度
            return GlyphBmp::empty(if n == u32::MAX {
                self.fallback_advance(ch, size)
            } else {
                advance
            });
        }

        let mut buf = vec![0u8; n as usize];
        let got = unsafe {
            GetGlyphOutlineW(
                self.dc,
                ch,
                GGO_GRAY8_BITMAP,
                &mut gm,
                n,
                Some(buf.as_mut_ptr() as *mut c_void),
                &mat,
            )
        };
        if got == 0 || got == u32::MAX {
            return GlyphBmp::empty(advance);
        }

        let w = gm.gmBlackBoxX as usize;
        let h = gm.gmBlackBoxY as usize;
        if w == 0 || h == 0 {
            return GlyphBmp::empty(advance);
        }
        // GGO_* 位图的每一扫描行按 4 字节对齐
        let stride = (w + 3) & !3;
        let mut cov = vec![0u8; w * h];
        if buf.len() >= stride * h {
            for y in 0..h {
                for x in 0..w {
                    // GDI 灰度抗锯齿给出 0..64 共 65 级，归一到 0..255
                    let v = buf[y * stride + x] as u32;
                    cov[y * w + x] = (((v * 255) / 64).min(255)) as u8;
                }
            }
        }
        GlyphBmp {
            w,
            h,
            left: gm.gmptGlyphOrigin.x as f32,
            // gmptGlyphOrigin.y 是"基线到黑盒顶"的距离（向上为正）
            top: -(gm.gmptGlyphOrigin.y as f32),
            advance,
            cov,
        }
    }

    /// 字形缺失时的兜底推进宽度（走 GDI 的字体链接/回退链）。
    fn fallback_advance(&self, ch: u32, size: f32) -> f32 {
        if ch > 0xFFFF {
            return size * 0.5;
        }
        let s = [ch as u16];
        let mut sz = SIZE::default();
        let ok = unsafe { GetTextExtentPoint32W(self.dc, &s, &mut sz) };
        if ok.as_bool() && sz.cx > 0 {
            sz.cx as f32
        } else {
            size * 0.5
        }
    }
}

/// 缓存字形位图按当前颜色做 premultiplied over 混合（定点运算）。
fn blit_glyph(px: &mut Pixmap, pen_x: f32, pen_y: f32, bmp: &GlyphBmp, color: Color) {
    if bmp.w == 0 || bmp.h == 0 || color.alpha() <= 0.0 {
        return;
    }
    let pw = px.width() as i32;
    let ph = px.height() as i32;
    // 落点取整：字形覆盖率是在整数网格上光栅化的，贴到整像素才清晰
    let ox = (pen_x + bmp.left).round() as i32;
    let oy = (pen_y + bmp.top).round() as i32;
    let cr = color.red() * 255.0;
    let cg = color.green() * 255.0;
    let cb = color.blue() * 255.0;
    let ca = color.alpha() * 255.0;
    let row_w = px.width();
    let d = px.data_mut();
    for gy in 0..bmp.h {
        let py = oy + gy as i32;
        if py < 0 || py >= ph {
            continue;
        }
        for gx in 0..bmp.w {
            let a = bmp.cov[gy * bmp.w + gx];
            if a == 0 {
                continue;
            }
            let px_x = ox + gx as i32;
            if px_x < 0 || px_x >= pw {
                continue;
            }
            // sa = 文字颜色 alpha × 字形覆盖率，0..255；inv 为目标色保留比例
            let sa = (ca * a as f32 / 255.0) as u32;
            let inv = 255 - sa;
            let i = ((py as u32 * row_w + px_x as u32) * 4) as usize;
            d[i] = ((cr as u32 * sa + d[i] as u32 * inv) / 255) as u8;
            d[i + 1] = ((cg as u32 * sa + d[i + 1] as u32 * inv) / 255) as u8;
            d[i + 2] = ((cb as u32 * sa + d[i + 2] as u32 * inv) / 255) as u8;
            d[i + 3] = (sa + d[i + 3] as u32 * inv / 255) as u8;
        }
    }
}
