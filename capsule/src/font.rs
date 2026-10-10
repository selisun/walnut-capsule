//! 字体加载与文本绘制。
//! 用 memmap 映射系统字体（不整文件读入内存），ab_glyph 光栅化字形，
//! 以 premultiplied "over" 混合写入 tiny-skia Pixmap。
//!
//! 字形位图按 (glyph_id, size) 缓存：界面字符集很小且每帧重复，
//! 首次出现的字符走一次 outline + 光栅化，之后只做内存 blit，
//! 避免每帧对几十上百个汉字重复跑贝塞尔光栅化。

use ab_glyph::{Font as _, FontArc, Glyph, PxScale, ScaleFont};
use std::cell::RefCell;
use std::collections::HashMap;
use tiny_skia::{Color, Pixmap};

const CANDIDATES: &[&str] = &[
    r"C:\Windows\Fonts\msyh.ttc",   // 微软雅黑
    r"C:\Windows\Fonts\simhei.ttf", // 黑体
    r"C:\Windows\Fonts\simsun.ttc", // 宋体
    r"C:\Windows\Fonts\segoeui.ttf",
];

/// 粗体候选：优先微软雅黑粗体，黑体兜底；都缺时回退到 regular 字形。
const BOLD_CANDIDATES: &[&str] = &[
    r"C:\Windows\Fonts\msyhbd.ttc", // 微软雅黑 Bold
    r"C:\Windows\Fonts\simhei.ttf", // 黑体
];

/// 字形缓存键：同一字符同一字号的位图与位置无关，可全局复用。
/// bold 区分粗细两套字形，避免缓存互相污染。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    id: u16,
    size_bits: u32,
    bold: bool,
}

/// 以笔尖（pen position，即 baseline 起点）为原点的字形 alpha 位图。
struct GlyphBmp {
    w: usize,
    h: usize,
    /// 位图左上角相对笔尖 x 的偏移（通常为负的左侧轴承）。
    left: f32,
    /// 位图左上角相对笔尖 y 的偏移（基线以上为负）。
    top: f32,
    advance: f32,
    /// w*h 覆盖率 alpha，0=透明。
    cov: Vec<u8>,
}

pub struct Font {
    inner: FontArc,
    bold: Option<FontArc>,
    cache: RefCell<HashMap<GlyphKey, GlyphBmp>>,
}

impl Font {
    pub fn load() -> Option<Font> {
        let inner = CANDIDATES.iter().find_map(|p| load_mapped(p))?;
        let bold = BOLD_CANDIDATES.iter().find_map(|p| load_mapped(p));
        Some(Font { inner, bold, cache: RefCell::new(HashMap::new()) })
    }

    pub fn ascent(&self, size: f32) -> f32 {
        self.inner.as_scaled(PxScale::from(size)).ascent()
    }

    /// 预热界面静态用字的字形缓存：在启动空闲期把每个字号下的常用字符
    /// 全部 outline + 光栅化一遍，避免展开首帧/首份面板构建时同步跑
    /// 几十个字形的冷光栅化（实测会造成毫秒级帧尖峰）。粗细两套都预热。
    pub fn preheat(&self) {
        // 界面静态字符串用到的汉字 + 数字/单位/标点 + 全字母（GPU 名/主机名多为 ASCII）
        const CHARS: &str = concat!(
            "使用率温度内存显存存储状态块盘休眠中运行未知已用容量硬盘读写收发连接后重等待数据正在断开",
            "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
            " .%°…·/-,",
        );
        const SIZES: [f32; 5] = [10.0, 12.5, 14.0, 16.5, 17.0];
        let black = Color::from_rgba8(0, 0, 0, 255);
        if let Some(mut tmp) = Pixmap::new(64, 64) {
            let mut buf = [0u8; 4];
            for &size in &SIZES {
                for c in CHARS.chars() {
                    let s = c.encode_utf8(&mut buf);
                    let _ = self.draw(&mut tmp, 0.0, size, size, s, black);
                }
            }
            if self.bold.is_some() {
                for &size in &SIZES {
                    for c in CHARS.chars() {
                        let s = c.encode_utf8(&mut buf);
                        let _ = self.draw_bold(&mut tmp, 0.0, size, size, s, black);
                    }
                }
            }
        }
    }

    /// 文本宽度（用于右对齐与截断）。
    pub fn measure(&self, size: f32, text: &str) -> f32 {
        let sf = self.inner.as_scaled(PxScale::from(size));
        text.chars().map(|c| sf.h_advance(sf.glyph_id(c))).sum()
    }

    /// 粗体文本宽度（粗体字宽略宽于常规，截断需按粗体测量）。
    pub fn measure_bold(&self, size: f32, text: &str) -> f32 {
        let font: &FontArc = match &self.bold {
            Some(b) => b,
            None => &self.inner,
        };
        let sf = font.as_scaled(PxScale::from(size));
        text.chars().map(|c| sf.h_advance(sf.glyph_id(c))).sum()
    }

    /// 在 (x, baseline) 处绘制文本，返回末端的 x。
    pub fn draw(&self, px: &mut Pixmap, x: f32, baseline: f32, size: f32, text: &str, color: Color) -> f32 {
        self.draw_impl(px, x, baseline, size, text, color, false)
    }

    /// 粗体文本绘制（无粗体字体时回退常规字形）。
    pub fn draw_bold(&self, px: &mut Pixmap, x: f32, baseline: f32, size: f32, text: &str, color: Color) -> f32 {
        self.draw_impl(px, x, baseline, size, text, color, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_impl(&self, px: &mut Pixmap, x: f32, baseline: f32, size: f32, text: &str, color: Color, bold: bool) -> f32 {
        let font: &FontArc = match (bold, &self.bold) {
            (true, Some(b)) => b,
            _ => &self.inner,
        };
        let sf = font.as_scaled(PxScale::from(size));
        let mut caret = x;
        for c in text.chars() {
            let id = sf.glyph_id(c);
            let key = GlyphKey { id: id.0, size_bits: size.to_bits(), bold };

            // 缓存缺失：在当前笔尖位置做一次 outline + 光栅化。
            if !self.cache.borrow().contains_key(&key) {
                let glyph: Glyph = id.with_scale_and_position(size, ab_glyph::point(caret, baseline));
                let advance = sf.h_advance(id);
                let bmp = if let Some(outlined) = font.outline_glyph(glyph) {
                    let bounds = outlined.px_bounds();
                    let w = bounds.width().ceil().max(0.0) as usize;
                    let h = bounds.height().ceil().max(0.0) as usize;
                    let mut cov = vec![0u8; w * h];
                    outlined.draw(|gx, gy, c| {
                        let idx = gy as usize * w + gx as usize;
                        let a = (c * 255.0).round() as u8;
                        if a > cov[idx] {
                            cov[idx] = a;
                        }
                    });
                    GlyphBmp {
                        w,
                        h,
                        left: bounds.min.x - caret,
                        top: bounds.min.y - baseline,
                        advance,
                        cov,
                    }
                } else {
                    // 空格等无轮廓字符：只占推进宽度。
                    GlyphBmp { w: 0, h: 0, left: 0.0, top: 0.0, advance, cov: Vec::new() }
                };
                self.cache.borrow_mut().insert(key, bmp);
            }

            {
                let cache = self.cache.borrow();
                let bmp = cache.get(&key).unwrap();
                blit_glyph(px, caret, baseline, bmp, color);
                caret += bmp.advance;
            }
        }
        caret
    }
}

/// 缓存字形位图按当前颜色做 premultiplied over 混合（定点运算）。
fn blit_glyph(px: &mut Pixmap, pen_x: f32, pen_y: f32, bmp: &GlyphBmp, color: Color) {
    if bmp.w == 0 || bmp.h == 0 || color.alpha() <= 0.0 {
        return;
    }
    let pw = px.width() as i32;
    let ph = px.height() as i32;
    let ox = (pen_x + bmp.left) as i32;
    let oy = (pen_y + bmp.top) as i32;
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

/// 内存映射字体文件并泄漏为 'static（进程生命周期内只用一次）。
fn load_mapped(path: &str) -> Option<FontArc> {
    let file = std::fs::File::open(path).ok()?;
    let map = unsafe { memmap2::Mmap::map(&file).ok()? };
    let slice: &'static [u8] = unsafe { std::mem::transmute::<&[u8], &'static [u8]>(&map[..]) };
    std::mem::forget(map);
    FontArc::try_from_slice(slice).ok()
}
