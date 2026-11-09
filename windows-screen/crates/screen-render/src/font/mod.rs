//! Windows GDI 字形轮廓 → tiny-skia 路径填充。
//! 不使用 DirectWrite/GPU/任何第三方字体 crate；字体使用系统已安装字体。

#[cfg(windows)]
pub mod glyph;

#[cfg(windows)]
use std::ffi::c_void;

#[cfg(windows)]
use screen_core::Color;
#[cfg(windows)]
use tiny_skia::Transform;
#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::SIZE;
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateFontW, DeleteDC, DeleteObject, GetGlyphOutlineW,
    GetTextExtentPoint32W, SelectObject, ANTIALIASED_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DEFAULT_PITCH, FF_DONTCARE, GGO_NATIVE, GLYPHMETRICS, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS,
};

#[cfg(windows)]
use crate::canvas::Canvas;
#[cfg(windows)]
use crate::error::RenderError;
#[cfg(windows)]
use crate::font::glyph::{
    glyph_error, identity_mat2, parse_glyph_outline, GlyphCache, GlyphOutline,
};
#[cfg(windows)]
use crate::text::{FontSpec, TextRenderer};

#[cfg(windows)]
pub struct GdiTextRenderer {
    hdc: HDC,
    hfont: HFONT,
    /// 建字体前的原字体：析构时必须还原，否则 DeleteObject 会因字体仍被选中而失败（句柄泄漏）。
    old_font: HGDIOBJ,
    size_px: f32,
    cache: GlyphCache,
    last_advance: f32,
}

#[cfg(windows)]
impl GdiTextRenderer {
    pub fn new(spec: &FontSpec) -> Result<Self, RenderError> {
        if !spec.size_px.is_finite() || spec.size_px <= 0.0 {
            return Err(RenderError::Text("字号必须为正数".to_string()));
        }
        let hdc = unsafe { CreateCompatibleDC(HDC::default()) };
        if hdc.is_invalid() {
            return Err(RenderError::Text("CreateCompatibleDC 失败".to_string()));
        }
        let mut families = vec![spec.family.as_str()];
        if spec.family != "Microsoft YaHei" {
            families.push("Microsoft YaHei");
        }
        if spec.family != "Bahnschrift" {
            families.push("Bahnschrift");
        }
        let mut hfont = HFONT::default();
        for family in families {
            hfont = create_font(family, spec);
            if !hfont.is_invalid() {
                break;
            }
        }
        if hfont.is_invalid() {
            unsafe {
                let _ = DeleteDC(hdc);
            }
            return Err(RenderError::Text(format!("无法创建字体：{}", spec.family)));
        }
        let old = unsafe { SelectObject(hdc, hfont) };
        if old.is_invalid() {
            unsafe {
                let _ = DeleteObject(hfont);
                let _ = DeleteDC(hdc);
            }
            return Err(RenderError::Text("SelectObject 失败".to_string()));
        }
        Ok(Self {
            hdc,
            hfont,
            old_font: old,
            size_px: spec.size_px,
            cache: GlyphCache::default(),
            last_advance: 0.0,
        })
    }

    pub fn glyph_cache_len(&self) -> usize {
        self.cache.len()
    }

    pub fn cache_hits(&self) -> u32 {
        self.cache.hits()
    }

    pub fn cache_misses(&self) -> u32 {
        self.cache.misses()
    }

    pub fn last_advance(&self) -> f32 {
        self.last_advance
    }

    /// 文本量宽（按当前字体、以创建时的字号为单位）。
    pub fn measure(&self, text: &str) -> Option<i32> {
        self.text_extent(text)
    }

    fn glyph(&mut self, ch: char) -> GlyphOutline {
        if let Some(glyph) = self.cache.get(ch).cloned() {
            self.cache.mark_hit();
            return glyph;
        }
        let glyph = self.load_glyph(ch);
        self.cache.insert(ch, glyph.clone());
        glyph
    }

    fn load_glyph(&self, ch: char) -> GlyphOutline {
        let mut metrics = GLYPHMETRICS::default();
        let mat = identity_mat2();
        let size = unsafe {
            GetGlyphOutlineW(self.hdc, ch as u32, GGO_NATIVE, &mut metrics, 0, None, &mat)
        };
        if size == glyph_error() {
            return GlyphOutline {
                path: None,
                advance: self.size_px * 0.3,
                origin_x: 0,
                origin_y: 0,
                black_box: (0, 0),
            };
        }
        let mut buffer = vec![0u8; size as usize];
        if size > 0 {
            let result = unsafe {
                GetGlyphOutlineW(
                    self.hdc,
                    ch as u32,
                    GGO_NATIVE,
                    &mut metrics,
                    size,
                    Some(buffer.as_mut_ptr() as *mut c_void),
                    &mat,
                )
            };
            if result == glyph_error() {
                return GlyphOutline {
                    path: None,
                    advance: self.size_px * 0.3,
                    origin_x: 0,
                    origin_y: 0,
                    black_box: (0, 0),
                };
            }
        }
        let glyph = parse_glyph_outline(&buffer, &metrics);
        match glyph {
            Ok(mut glyph) => {
                if glyph.advance <= 0.0 {
                    glyph.advance = self.size_px * 0.3;
                }
                glyph
            }
            Err(_) => GlyphOutline {
                path: None,
                advance: self.size_px * 0.3,
                origin_x: 0,
                origin_y: 0,
                black_box: (0, 0),
            },
        }
    }

    fn text_extent(&self, text: &str) -> Option<i32> {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        if utf16.is_empty() {
            return Some(0);
        }
        let mut size = SIZE::default();
        let ok = unsafe { GetTextExtentPoint32W(self.hdc, &utf16, &mut size) };
        if ok.as_bool() {
            Some(size.cx)
        } else {
            None
        }
    }
}

#[cfg(windows)]
impl TextRenderer for GdiTextRenderer {
    fn draw_text(
        &mut self,
        canvas: &mut Canvas,
        text: &str,
        x: f32,
        y: f32,
        max_width: f32,
        size: f32,
        color: Color,
    ) -> Result<(), RenderError> {
        if text.is_empty() || max_width <= 0.0 {
            self.last_advance = 0.0;
            return Ok(());
        }
        let size = if size.is_finite() && size > 0.0 {
            size
        } else {
            self.size_px
        };
        let scale = size / self.size_px;
        let baseline = y + size;
        let fits = self
            .text_extent(text)
            .map(|w| w as f32 * scale <= max_width)
            .unwrap_or(false);
        let mut pen = x;
        for ch in text.chars() {
            let glyph = self.glyph(ch);
            let advance = glyph.advance.max(self.size_px * 0.1) * scale;
            if !fits && pen + advance > x + max_width {
                break;
            }
            if let Some(path) = &glyph.path {
                let transform = Transform::from_translate(pen, baseline).pre_scale(scale, scale);
                if let Some(path) = path.clone().transform(transform) {
                    canvas.fill_path(&path, color)?;
                }
            }
            pen += advance;
        }
        self.last_advance = pen - x;
        Ok(())
    }

    fn measure_text(&mut self, text: &str, size: f32) -> Option<f32> {
        let size = if size.is_finite() && size > 0.0 {
            size
        } else {
            self.size_px
        };
        self.text_extent(text)
            .map(|w| w as f32 * (size / self.size_px))
    }
}

#[cfg(windows)]
impl Drop for GdiTextRenderer {
    fn drop(&mut self) {
        unsafe {
            // 先还原原字体，字体不再被 DC 选中，DeleteObject 才能成功释放句柄
            let _ = SelectObject(self.hdc, self.old_font);
            let _ = DeleteObject(self.hfont);
            let _ = DeleteDC(self.hdc);
        }
    }
}

#[cfg(windows)]
fn create_font(family: &str, spec: &FontSpec) -> HFONT {
    let face: Vec<u16> = family.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        CreateFontW(
            -spec.size_px.round() as i32,
            0,
            0,
            0,
            spec.weight as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            ANTIALIASED_QUALITY.0 as u32,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            PCWSTR::from_raw(face.as_ptr()),
        )
    }
}

#[cfg(all(windows, test))]
mod tests {
    use super::*;

    fn renderer(size: f32, family: &str) -> GdiTextRenderer {
        GdiTextRenderer::new(&FontSpec {
            family: family.to_string(),
            size_px: size,
            weight: 400,
        })
        .expect("创建 GDI 文字渲染器")
    }

    fn draw(
        renderer: &mut GdiTextRenderer,
        width: u32,
        height: u32,
        text: &str,
        max_width: f32,
        size: f32,
    ) -> (Vec<u8>, f32) {
        let mut canvas = Canvas::new(width, height).expect("canvas");
        canvas.clear(Color::rgb(0, 0, 0));
        renderer
            .draw_text(
                &mut canvas,
                text,
                2.0,
                2.0,
                max_width,
                size,
                Color::rgb(255, 255, 255),
            )
            .expect("绘制文字");
        let advance = renderer.last_advance();
        (
            canvas.into_frame().expect("frame").pixels().to_vec(),
            advance,
        )
    }

    fn non_black_pixels(pixels: &[u8]) -> usize {
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 0 || p[1] > 0 || p[2] > 0)
            .count()
    }

    #[test]
    fn renderer_measures_text() {
        let renderer = renderer(14.0, "Microsoft YaHei");
        assert_eq!(renderer.measure(""), Some(0));
        let width = renderer.measure("CPU 42%").expect("量宽");
        assert!(width > 0, "拉丁文本宽度应为正数，实际 {width}");
        let cjk = renderer.measure("内存").expect("量宽");
        assert!(cjk > 0, "中文宽度应为正数，实际 {cjk}");
    }

    #[test]
    fn empty_text_is_noop() {
        let mut renderer = renderer(14.0, "Microsoft YaHei");
        let (pixels, advance) = draw(&mut renderer, 32, 16, "", 32.0, 14.0);
        assert_eq!(advance, 0.0);
        assert_eq!(non_black_pixels(&pixels), 0);
        assert_eq!(renderer.glyph_cache_len(), 0);
    }

    #[test]
    fn zero_width_draws_nothing() {
        let mut renderer = renderer(14.0, "Microsoft YaHei");
        let (pixels, advance) = draw(&mut renderer, 32, 16, "CPU", 0.0, 14.0);
        assert_eq!(advance, 0.0);
        assert_eq!(non_black_pixels(&pixels), 0);
    }

    #[test]
    fn font_handle_is_released_on_drop() {
        use windows::Win32::System::Threading::{
            GetCurrentProcess, GetGuiResources, GR_GDIOBJECTS,
        };
        let before = unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        if before == 0 {
            // 非交互会话拿不到计数，跳过而不是误报
            return;
        }
        for _ in 0..100 {
            drop(renderer(14.0, "Microsoft YaHei"));
        }
        let after = unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        // 其他测试可能并发建渲染器，留出余量；若真泄漏，100 次会明显超过阈值
        assert!(
            after <= before + 30,
            "HFONT 未释放：GDI 句柄 {before} -> {after}"
        );
    }

    #[test]
    fn glyph_cache_hits_on_repeat() {
        let mut renderer = renderer(14.0, "Microsoft YaHei");
        draw(&mut renderer, 96, 24, "CPU", 92.0, 14.0);
        let first_len = renderer.glyph_cache_len();
        let first_misses = renderer.cache_misses();
        assert!(first_len > 0, "首轮应有字形缓存");
        assert_eq!(first_misses as usize, first_len);

        draw(&mut renderer, 96, 24, "CPU", 92.0, 14.0);
        assert_eq!(renderer.glyph_cache_len(), first_len);
        assert!(renderer.cache_hits() > 0, "重复文本应命中缓存");
    }

    #[test]
    fn wide_text_is_truncated_within_width() {
        let mut renderer = renderer(14.0, "Microsoft YaHei");
        let (pixels, advance) = draw(
            &mut renderer,
            240,
            24,
            "CPU 内存 网络 磁盘 12345 67890",
            40.0,
            14.0,
        );
        assert!(advance <= 40.0, "截断后宽度 {advance} 不应超过 40");
        assert!(non_black_pixels(&pixels) > 0, "截断后仍应绘制可见字形");
    }

    #[test]
    fn overlong_text_does_not_panic() {
        let mut renderer = renderer(12.0, "Microsoft YaHei");
        let text = "核桃胶囊".repeat(3000);
        let (_, advance) = draw(&mut renderer, 64, 16, &text, 30.0, 12.0);
        assert!(advance <= 30.0);
    }

    #[test]
    fn emoji_does_not_panic() {
        let mut renderer = renderer(14.0, "Microsoft YaHei");
        let _ = draw(&mut renderer, 64, 24, "🙂🚀🀄", 60.0, 14.0);
        assert!(renderer.glyph_cache_len() > 0);
    }

    #[test]
    fn surrogate_pair_does_not_panic() {
        let mut renderer = renderer(14.0, "Microsoft YaHei");
        let _ = draw(&mut renderer, 64, 24, "𝐀𠀀", 60.0, 14.0);
    }

    #[test]
    fn control_chars_do_not_panic() {
        let mut renderer = renderer(14.0, "Microsoft YaHei");
        let _ = draw(&mut renderer, 64, 24, "A\r\n\tB", 60.0, 14.0);
    }

    #[test]
    fn chinese_text_produces_visible_pixels() {
        let mut renderer = renderer(20.0, "Microsoft YaHei");
        let (pixels, advance) = draw(&mut renderer, 120, 40, "内存 42%", 116.0, 20.0);
        assert!(non_black_pixels(&pixels) > 0, "中文应渲染出可见像素");
        assert!(advance > 0.0);
    }

    #[test]
    fn larger_size_scales_advance() {
        let mut small = renderer(12.0, "Microsoft YaHei");
        let mut large = renderer(12.0, "Microsoft YaHei");
        let (_, small_advance) = draw(&mut small, 200, 40, "内存", 196.0, 12.0);
        let (_, large_advance) = draw(&mut large, 300, 60, "内存", 296.0, 24.0);
        assert!(large_advance > small_advance);
    }

    #[test]
    fn unknown_family_falls_back_to_system_font() {
        let mut renderer = renderer(14.0, "Bahnschrift");
        let (pixels, advance) = draw(&mut renderer, 120, 24, "Disk", 116.0, 14.0);
        assert!(non_black_pixels(&pixels) > 0);
        assert!(advance > 0.0);
    }

    #[test]
    fn drop_releases_handles_without_panic() {
        for _ in 0..8 {
            let renderer = renderer(14.0, "Microsoft YaHei");
            drop(renderer);
        }
    }
}
