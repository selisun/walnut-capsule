//! 渲染管线：背景 + 卡片底 + 可选标签；`render_with_metrics` 绑定真实指标数值。
//!
//! 缺失指标不画卡、不画假数据；数值格式化由 `metrics` 模块负责。

use screen_core::{
    ComponentKind, FrameBuffer, ImageAnchor, ImageFit, LayoutTree, ScreenProfile, ThemeConfig,
};

use crate::canvas::Canvas;
use crate::error::RenderError;
use crate::image::BackgroundImage;
use crate::metrics::{default_label, MetricView};
use crate::text::TextRenderer;

pub fn render(
    profile: &ScreenProfile,
    theme: &ThemeConfig,
    layout: &LayoutTree,
    text: Option<&mut dyn TextRenderer>,
) -> Result<FrameBuffer, RenderError> {
    render_with_bg(profile, theme, layout, text, None)
}

pub fn render_with_bg(
    profile: &ScreenProfile,
    theme: &ThemeConfig,
    layout: &LayoutTree,
    mut text: Option<&mut dyn TextRenderer>,
    background: Option<&BackgroundImage>,
) -> Result<FrameBuffer, RenderError> {
    if layout.profile_id != profile.id {
        return Err(RenderError::InvalidLayout(format!(
            "布局 Profile 不匹配：{} != {}",
            layout.profile_id, profile.id
        )));
    }
    layout
        .validate(profile.width as f32, profile.height as f32)
        .map_err(|e| RenderError::InvalidLayout(e.to_string()))?;

    let mut canvas = paint_background(profile, theme, background)?;

    for item in &layout.items {
        canvas.fill_rect(item.rect, theme.palette.card)?;
        // 卡片顶部强调条（纯装饰，不表达数据）
        let bar = screen_core::Rect::new(
            item.rect.x,
            item.rect.y,
            item.rect.w,
            (2.0_f32).min(item.rect.h),
        );
        canvas.fill_rect(bar, theme.palette.accent)?;

        if let Some(renderer) = text.as_deref_mut() {
            let label = theme
                .components
                .styles
                .get(item.component.as_str())
                .and_then(|s| s.label.as_deref())
                .unwrap_or_else(|| item.component.as_str());
            renderer.draw_text(
                &mut canvas,
                label,
                item.rect.x + 8.0,
                item.rect.y + layout.font_px + 8.0,
                (item.rect.w - 16.0).max(0.0),
                layout.font_px,
                theme.palette.text,
            )?;
        }
    }

    canvas.into_frame()
}

/// 按指标视图渲染：标题 + 主数值（大字）+ 副信息（小字）。
/// 只绘制 `metrics` 中存在的卡片；`layout` 里没有对应项的视图被忽略，绝不画空白格。
pub fn render_with_metrics(
    profile: &ScreenProfile,
    theme: &ThemeConfig,
    layout: &LayoutTree,
    metrics: &[(ComponentKind, MetricView)],
    text: Option<&mut dyn TextRenderer>,
) -> Result<FrameBuffer, RenderError> {
    render_with_metrics_bg(profile, theme, layout, metrics, text, None)
}

pub fn render_with_metrics_bg(
    profile: &ScreenProfile,
    theme: &ThemeConfig,
    layout: &LayoutTree,
    metrics: &[(ComponentKind, MetricView)],
    mut text: Option<&mut dyn TextRenderer>,
    background: Option<&BackgroundImage>,
) -> Result<FrameBuffer, RenderError> {
    if layout.profile_id != profile.id {
        return Err(RenderError::InvalidLayout(format!(
            "布局 Profile 不匹配：{} != {}",
            layout.profile_id, profile.id
        )));
    }
    layout
        .validate(profile.width as f32, profile.height as f32)
        .map_err(|e| RenderError::InvalidLayout(e.to_string()))?;

    let mut canvas = paint_background(profile, theme, background)?;

    for (kind, view) in metrics {
        let Some(item) = layout.items.iter().find(|item| item.component == *kind) else {
            continue;
        };
        canvas.fill_rect(item.rect, theme.palette.card)?;
        let bar = screen_core::Rect::new(
            item.rect.x,
            item.rect.y,
            item.rect.w,
            (2.0_f32).min(item.rect.h),
        );
        canvas.fill_rect(bar, theme.palette.accent)?;

        let Some(renderer) = text.as_deref_mut() else {
            continue;
        };
        let label = theme
            .components
            .styles
            .get(kind.as_str())
            .and_then(|style| style.label.as_deref())
            .unwrap_or_else(|| default_label(*kind));
        let accent = component_accent(theme, kind);
        let title_px = layout.font_px;
        let value_px = (title_px * 1.7).min(item.rect.h * 0.38).max(title_px);
        let x = item.rect.x + 8.0;
        let max_width = (item.rect.w - 16.0).max(0.0);
        let mut y = item.rect.y + 6.0;

        renderer.draw_text(
            &mut canvas,
            label,
            x,
            y,
            max_width,
            title_px,
            theme.palette.dim,
        )?;
        y += title_px + 4.0;
        renderer.draw_text(
            &mut canvas,
            &view.primary(),
            x,
            y,
            max_width,
            value_px,
            accent,
        )?;
        y += value_px + 2.0;
        let secondary = view.secondary();
        if !secondary.is_empty() && y + title_px <= item.rect.y + item.rect.h {
            renderer.draw_text(
                &mut canvas,
                &secondary,
                x,
                y,
                max_width,
                (title_px * 0.9).max(9.0),
                theme.palette.dim,
            )?;
        }
    }

    canvas.into_frame()
}

/// 占位画面：只画背景与一条提示文字（等待客户端/数据过期/版本不兼容）。
pub fn render_notice(
    profile: &ScreenProfile,
    theme: &ThemeConfig,
    notice: &str,
    text: Option<&mut dyn TextRenderer>,
) -> Result<FrameBuffer, RenderError> {
    render_notice_bg(profile, theme, notice, text, None)
}

pub fn render_notice_bg(
    profile: &ScreenProfile,
    theme: &ThemeConfig,
    notice: &str,
    text: Option<&mut dyn TextRenderer>,
    background: Option<&BackgroundImage>,
) -> Result<FrameBuffer, RenderError> {
    let mut canvas = paint_background(profile, theme, background)?;
    if let Some(renderer) = text {
        let size = (profile.short_edge as f32 * 0.08).max(14.0);
        let y = profile.height as f32 / 2.0 - size / 2.0;
        renderer.draw_text(
            &mut canvas,
            notice,
            12.0,
            y,
            (profile.width as f32 - 24.0).max(0.0),
            size,
            theme.palette.dim,
        )?;
    }
    canvas.into_frame()
}

/// 背景：无图时用 fallback_color；有图时按 fit/anchor 缩放，再按主题 dim 压暗。
fn paint_background(
    profile: &ScreenProfile,
    theme: &ThemeConfig,
    background: Option<&BackgroundImage>,
) -> Result<Canvas, RenderError> {
    let mut canvas = Canvas::new(profile.width, profile.height)?;
    let Some(image) = background else {
        canvas.clear(theme.background.fallback_color);
        return Ok(canvas);
    };
    let src_w = image.width() as f32;
    let src_h = image.height() as f32;
    let sx = profile.width as f32 / src_w;
    let sy = profile.height as f32 / src_h;
    let scale = match theme.background.fit {
        ImageFit::Cover => sx.max(sy),
        ImageFit::Contain => sx.min(sy),
    };
    let drawn_w = src_w * scale;
    let drawn_h = src_h * scale;
    let dx = (profile.width as f32 - drawn_w) / 2.0;
    let dy = match theme.background.anchor {
        ImageAnchor::Center => (profile.height as f32 - drawn_h) / 2.0,
        ImageAnchor::Top => 0.0,
        ImageAnchor::Bottom => profile.height as f32 - drawn_h,
    };
    canvas.draw_pixmap_scaled(image.pixmap(), dx, dy, scale)?;

    let dim = theme.background.dim.clamp(0.0, 1.0);
    if dim > 0.0 {
        let overlay = screen_core::Color::new(0, 0, 0, (dim * 255.0).round() as u8);
        canvas.fill_rect(
            screen_core::Rect::new(0.0, 0.0, profile.width as f32, profile.height as f32),
            overlay,
        )?;
    }
    Ok(canvas)
}

fn component_accent(theme: &ThemeConfig, kind: &ComponentKind) -> screen_core::Color {
    let key = theme
        .components
        .styles
        .get(kind.as_str())
        .and_then(|style| style.accent.as_deref())
        .unwrap_or("accent");
    match key {
        "text" => theme.palette.text,
        "dim" => theme.palette.dim,
        "accent" => theme.palette.accent,
        "warn" => theme.palette.warn,
        "danger" => theme.palette.danger,
        _ => theme.palette.accent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use screen_core::{Color, LayoutItem, Orientation, Palette, Rect};
    use screen_layout::{build, TemplateKind};

    fn palette() -> Palette {
        Palette {
            bg: Color::rgb(16, 20, 24),
            card: Color::rgb(27, 32, 40),
            text: Color::rgb(232, 237, 242),
            dim: Color::rgb(138, 148, 166),
            accent: Color::rgb(55, 214, 122),
            warn: Color::rgb(255, 176, 32),
            danger: Color::rgb(255, 90, 95),
        }
    }

    fn theme() -> ThemeConfig {
        ThemeConfig {
            schema: ThemeConfig::SCHEMA_V1.to_string(),
            id: "t".into(),
            name: "测试".into(),
            author: None,
            version: "1.0.0".into(),
            palette: palette(),
            typography: Default::default(),
            background: Default::default(),
            components: Default::default(),
            layout: Default::default(),
            resources: Vec::new(),
        }
    }

    fn profile() -> ScreenProfile {
        ScreenProfile::new("3.5-l", 480, 320, Orientation::Landscape, true).expect("profile")
    }

    fn layout(profile: &ScreenProfile) -> LayoutTree {
        build(
            profile,
            &screen_core::Components::default().visible,
            TemplateKind::Landscape4Col,
            1.0,
        )
        .expect("layout")
    }

    struct FailingText;
    impl crate::text::TextRenderer for FailingText {
        fn draw_text(
            &mut self,
            _canvas: &mut Canvas,
            _text: &str,
            _x: f32,
            _y: f32,
            _max_width: f32,
            _size: f32,
            _color: Color,
        ) -> Result<(), RenderError> {
            Err(RenderError::TextUnavailable)
        }
    }

    fn solid_background(w: u32, h: u32, rgb: (u8, u8, u8)) -> BackgroundImage {
        let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("pixmap");
        pixmap.fill(tiny_skia::Color::from_rgba8(rgb.0, rgb.1, rgb.2, 255));
        let png = pixmap.encode_png().expect("encode");
        BackgroundImage::from_png(&png).expect("decode")
    }

    #[test]
    fn background_image_covers_canvas_without_cards() {
        let p = profile();
        let bg = solid_background(2, 3, (200, 10, 10));
        let mut theme = theme();
        theme.background.dim = 0.0;
        let frame =
            render_with_metrics_bg(&p, &theme, &layout(&p), &[], None, Some(&bg)).expect("render");
        assert_eq!(&frame.pixels()[0..4], &[200, 10, 10, 255]);
    }

    #[test]
    fn background_dim_darkens_image() {
        let p = profile();
        let bg = solid_background(2, 3, (200, 10, 10));
        let mut theme = theme();
        theme.background.dim = 0.5;
        let frame =
            render_with_metrics_bg(&p, &theme, &layout(&p), &[], None, Some(&bg)).expect("render");
        let px = &frame.pixels()[0..4];
        assert!(px[0] >= 95 && px[0] <= 105, "实际 {px:?}");
        assert!(px[1] <= 10 && px[2] <= 10, "实际 {px:?}");
    }

    #[test]
    fn empty_background_uses_fallback_color() {
        let p = profile();
        let frame = render(&p, &theme(), &layout(&p), None).expect("render");
        assert_eq!(&frame.pixels()[0..4], &[16, 20, 24, 255]);
    }

    #[test]
    fn renders_background_and_cards_without_text() {
        let p = profile();
        let tree = layout(&p);
        let frame = render(&p, &theme(), &tree, None).expect("render");
        assert_eq!((frame.width(), frame.height()), (480, 320));
        assert_eq!(frame.pixels().len(), 480 * 320 * 4);
        // 左上角背景
        assert_eq!(&frame.pixels()[0..4], &[16, 20, 24, 255]);
        // 第一张卡片内部（避开顶部强调条）
        let item = &tree.items[0].rect;
        let x = item.x as u32 + 4;
        let y = item.y as u32 + 10;
        let idx = ((y * 480 + x) * 4) as usize;
        assert_eq!(&frame.pixels()[idx..idx + 4], &[27, 32, 40, 255]);
    }

    #[test]
    fn text_unavailable_error_is_propagated() {
        let p = profile();
        let tree = layout(&p);
        let mut text = FailingText;
        let err = render(&p, &theme(), &tree, Some(&mut text)).unwrap_err();
        assert_eq!(err, RenderError::TextUnavailable);
    }

    #[test]
    fn profile_mismatch_is_rejected() {
        let p = profile();
        let mut tree = layout(&p);
        tree.profile_id = "other".into();
        assert!(render(&p, &theme(), &tree, None).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn render_draws_real_glyphs_when_text_is_available() {
        let p = profile();
        let tree = layout(&p);
        let mut text = crate::text::create_text_renderer(&crate::text::FontSpec::default())
            .expect("GDI 文字渲染器");
        let frame = render(&p, &theme(), &tree, Some(text.as_mut())).expect("render");
        let bright = frame
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[0] > 200 && px[1] > 200 && px[2] > 200)
            .count();
        assert!(bright > 0, "管线应绘制出真实字形像素");
    }

    fn card(component: ComponentKind, x: f32, y: f32, w: f32, h: f32) -> LayoutItem {
        LayoutItem {
            component,
            rect: Rect::new(x, y, w, h),
        }
    }

    fn tree(profile: &ScreenProfile, items: Vec<LayoutItem>) -> LayoutTree {
        LayoutTree {
            profile_id: profile.id.clone(),
            template: "landscape-4col".to_string(),
            font_px: 14.0,
            gap: 8.0,
            items,
        }
    }

    #[test]
    fn empty_metrics_render_only_background() {
        let p = profile();
        let tree = tree(&p, Vec::new());
        let frame = render_with_metrics(&p, &theme(), &tree, &[], None).expect("render");
        assert!(
            frame
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|px| *px == [16, 20, 24, 255]),
            "无指标时只应保留背景，不得出现空白卡片"
        );
    }

    #[test]
    fn metric_without_layout_item_is_not_drawn() {
        let p = profile();
        let cpu = card(ComponentKind::Cpu, 8.0, 8.0, 140.0, 100.0);
        // 布局里预留了内存卡位置，但 metrics 只有 CPU：内存位置必须保持背景色。
        let memory = card(ComponentKind::Memory, 160.0, 8.0, 140.0, 100.0);
        let tree = tree(&p, vec![cpu.clone(), memory.clone()]);
        let bound = vec![(
            ComponentKind::Cpu,
            MetricView::Cpu {
                usage: Some(42.0),
                temp: None,
            },
        )];
        let frame = render_with_metrics(&p, &theme(), &tree, &bound, None).expect("render");

        let x = cpu.rect.x as u32 + 4;
        let y = cpu.rect.y as u32 + 40;
        let idx = ((y * 480 + x) * 4) as usize;
        assert_eq!(&frame.pixels()[idx..idx + 4], &[27, 32, 40, 255]);

        let x = memory.rect.x as u32 + 4;
        let y = memory.rect.y as u32 + 40;
        let idx = ((y * 480 + x) * 4) as usize;
        assert_eq!(&frame.pixels()[idx..idx + 4], &[16, 20, 24, 255]);
    }

    #[test]
    fn missing_usage_renders_dash_instead_of_zero() {
        let p = profile();
        let cpu = card(ComponentKind::Cpu, 8.0, 8.0, 200.0, 100.0);
        let tree = tree(&p, vec![cpu]);
        let bound = vec![(
            ComponentKind::Cpu,
            MetricView::Cpu {
                usage: None,
                temp: None,
            },
        )];
        let frame = render_with_metrics(&p, &theme(), &tree, &bound, None).expect("render");
        assert_eq!((frame.width(), frame.height()), (480, 320));
    }

    #[cfg(windows)]
    #[test]
    fn metrics_render_draws_title_and_value() {
        let p = profile();
        let cpu = card(ComponentKind::Cpu, 8.0, 8.0, 200.0, 110.0);
        let tree = tree(&p, vec![cpu]);
        let bound = vec![(
            ComponentKind::Cpu,
            MetricView::Cpu {
                usage: Some(42.0),
                temp: Some(45.0),
            },
        )];
        let mut text = crate::text::create_text_renderer(&crate::text::FontSpec::default())
            .expect("GDI 文字渲染器");
        let frame =
            render_with_metrics(&p, &theme(), &tree, &bound, Some(text.as_mut())).expect("render");
        let bright = frame
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[0] as u32 + px[1] as u32 + px[2] as u32 > 120)
            .count();
        assert!(bright > 0, "指标卡应绘制标题/数值/副信息");
    }
}
