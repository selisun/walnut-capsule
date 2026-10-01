//! 字体加载与文本绘制。
//! 用 memmap 映射系统字体（不整文件读入内存），ab_glyph 光栅化字形，
//! 以 premultiplied "over" 混合写入 tiny-skia Pixmap。

use ab_glyph::{Font as _, FontArc, Glyph, PxScale, ScaleFont};
use tiny_skia::{Color, Pixmap};

const CANDIDATES: &[&str] = &[
    r"C:\Windows\Fonts\msyh.ttc",   // 微软雅黑
    r"C:\Windows\Fonts\simhei.ttf", // 黑体
    r"C:\Windows\Fonts\simsun.ttc", // 宋体
    r"C:\Windows\Fonts\segoeui.ttf",
];

pub struct Font {
    inner: FontArc,
}

impl Font {
    pub fn load() -> Option<Font> {
        for path in CANDIDATES {
            if let Some(f) = load_mapped(path) {
                return Some(f);
            }
        }
        None
    }

    pub fn ascent(&self, size: f32) -> f32 {
        self.inner.as_scaled(PxScale::from(size)).ascent()
    }

    /// 在 (x, baseline) 处绘制文本，返回末端的 x。
    pub fn draw(&self, px: &mut Pixmap, x: f32, baseline: f32, size: f32, text: &str, color: Color) -> f32 {
        let sf = self.inner.as_scaled(PxScale::from(size));
        let mut caret = x;
        for c in text.chars() {
            let glyph: Glyph = sf
                .glyph_id(c)
                .with_scale_and_position(size, ab_glyph::point(caret, baseline));
            caret += sf.h_advance(glyph.id);
            let Some(outlined) = self.inner.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, cov| {
                let px_x = bounds.min.x as i32 + gx as i32;
                let px_y = bounds.min.y as i32 + gy as i32;
                blend(px, px_x, px_y, color, cov);
            });
        }
        caret
    }
}

/// premultiplied over：src 为纯色 × coverage。
fn blend(px: &mut Pixmap, x: i32, y: i32, color: Color, cov: f32) {
    if x < 0 || y < 0 || x >= px.width() as i32 || y >= px.height() as i32 || cov <= 0.0 {
        return;
    }
    let sa = color.alpha() * cov;
    let i = ((y as u32 * px.width() + x as u32) * 4) as usize;
    let d = px.data_mut();
    let inv = 1.0 - sa;
    d[i] = (color.red() * sa * 255.0 + d[i] as f32 * inv) as u8;
    d[i + 1] = (color.green() * sa * 255.0 + d[i + 1] as f32 * inv) as u8;
    d[i + 2] = (color.blue() * sa * 255.0 + d[i + 2] as f32 * inv) as u8;
    d[i + 3] = (sa * 255.0 + d[i + 3] as f32 * inv) as u8;
}

/// 内存映射字体文件并泄漏为 'static（进程生命周期内只用一次）。
fn load_mapped(path: &str) -> Option<Font> {
    let file = std::fs::File::open(path).ok()?;
    let map = unsafe { memmap2::Mmap::map(&file).ok()? };
    let slice: &'static [u8] = unsafe { std::mem::transmute::<&[u8], &'static [u8]>(&map[..]) };
    std::mem::forget(map);
    let inner = FontArc::try_from_slice(slice).ok()?;
    Some(Font { inner })
}
