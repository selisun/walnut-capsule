//! 网格布局计算。坐标以逻辑像素为单位，允许任意分辨率。

use screen_core::{ComponentKind, LayoutItem, LayoutTree, Rect, ScreenProfile};

use crate::error::LayoutError;
use crate::template::TemplateKind;

const BASE_SHORT_EDGE: f32 = 320.0;
const BASE_FONT_PX: f32 = 14.0;
const BASE_GAP: f32 = 8.0;
const MIN_CARD_W: f32 = 64.0;
const MIN_CARD_H: f32 = 48.0;
const MIN_FONT_PX: f32 = 9.0;

/// 按 Profile + 组件列表生成网格布局。
pub fn build(
    profile: &ScreenProfile,
    visible: &[ComponentKind],
    template: TemplateKind,
    font_scale: f32,
) -> Result<LayoutTree, LayoutError> {
    if visible.is_empty() {
        return Err(LayoutError::NoComponents);
    }
    if !font_scale.is_finite() || !(0.5..=3.0).contains(&font_scale) {
        return Err(LayoutError::Validation("font_scale 越界".to_string()));
    }

    let scale = (profile.short_edge as f32 / BASE_SHORT_EDGE).clamp(0.75, 3.0);
    let gap = (BASE_GAP * scale).round().max(4.0);
    let margin = (BASE_GAP * scale).round().max(4.0);
    let cols = template.cols();
    let rows = visible.len().div_ceil(cols);

    let usable_w = profile.width as f32 - margin * 2.0 - gap * (cols as f32 - 1.0);
    let usable_h = profile.height as f32 - margin * 2.0 - gap * (rows as f32 - 1.0);
    let cell_w = usable_w / cols as f32;
    let cell_h = usable_h / rows as f32;

    if cell_w < MIN_CARD_W || cell_h < MIN_CARD_H {
        return Err(LayoutError::TooSmall {
            cell_w: cell_w as i32,
            cell_h: cell_h as i32,
        });
    }

    let font_px = (BASE_FONT_PX * scale * font_scale)
        .clamp(MIN_FONT_PX, cell_h * 0.35)
        .max(MIN_FONT_PX);

    let mut items = Vec::with_capacity(visible.len());
    for (i, component) in visible.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let x = margin + col as f32 * (cell_w + gap);
        let y = margin + row as f32 * (cell_h + gap);
        items.push(LayoutItem {
            component: *component,
            rect: Rect::new(x, y, cell_w, cell_h),
        });
    }

    let tree = LayoutTree {
        profile_id: profile.id.clone(),
        template: template.name().to_string(),
        font_px,
        gap,
        items,
    };
    tree.validate(profile.width as f32, profile.height as f32)
        .map_err(|e| LayoutError::Validation(e.to_string()))?;
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use screen_core::{Orientation, SizeClass};

    fn components() -> Vec<ComponentKind> {
        vec![
            ComponentKind::Cpu,
            ComponentKind::Memory,
            ComponentKind::Gpu,
            ComponentKind::Network,
            ComponentKind::Disk,
        ]
    }

    fn profile(id: &str, w: u32, h: u32, portrait: bool) -> ScreenProfile {
        let orientation = if portrait {
            Orientation::Portrait
        } else {
            Orientation::Landscape
        };
        ScreenProfile::new(id, w, h, orientation, !id.contains("future")).expect("profile")
    }

    #[test]
    fn landscape_uses_four_columns() {
        let p = profile("3.5l", 480, 320, false);
        let tree = build(&p, &components(), TemplateKind::Landscape4Col, 1.0).expect("layout");
        assert_eq!(tree.items.len(), 5);
        assert_eq!(tree.template, "landscape-4col");
        // 第一行 4 个，第二行 1 个
        assert!(tree.items[0].rect.y < tree.items[4].rect.y);
        assert_eq!(tree.items[4].rect.x, tree.items[0].rect.x);
    }

    #[test]
    fn portrait_uses_two_columns() {
        let p = profile("3.5p", 320, 480, true);
        let tree = build(&p, &components(), TemplateKind::Portrait2Col, 1.0).expect("layout");
        assert_eq!(tree.items.len(), 5);
        assert_eq!(tree.template, "portrait-2col");
        assert_eq!(tree.items[0].rect.y, tree.items[1].rect.y);
        assert!(tree.items[2].rect.y > tree.items[0].rect.y);
    }

    #[test]
    fn ultra_wide_uses_six_columns() {
        let p = profile("9.2", 1920, 462, false);
        assert_eq!(p.size_class, SizeClass::UltraWide);
        let tree = build(&p, &components(), TemplateKind::Wide6Col, 1.0).expect("layout");
        assert_eq!(tree.items.len(), 5);
        for item in &tree.items {
            assert!(item.rect.right() <= 1920.0);
            assert!(item.rect.bottom() <= 462.0);
        }
    }

    #[test]
    fn square_profile_works() {
        let p = profile("2.1", 480, 480, false);
        let tree = build(&p, &components(), TemplateKind::Square2x2, 1.0).expect("layout");
        assert!(tree.validate(480.0, 480.0).is_ok());
    }

    #[test]
    fn tiny_screen_is_rejected_instead_of_overflowing() {
        let p = profile("tiny", 100, 80, false);
        let err = build(&p, &components(), TemplateKind::Landscape4Col, 1.0).unwrap_err();
        assert!(matches!(err, LayoutError::TooSmall { .. }));
    }

    #[test]
    fn no_components_is_rejected() {
        let p = profile("3.5l", 480, 320, false);
        assert_eq!(
            build(&p, &[], TemplateKind::Landscape4Col, 1.0).unwrap_err(),
            LayoutError::NoComponents
        );
    }

    #[test]
    fn invalid_font_scale_is_rejected() {
        let p = profile("3.5l", 480, 320, false);
        assert!(build(&p, &components(), TemplateKind::Landscape4Col, 10.0).is_err());
    }

    #[test]
    fn font_size_respects_minimum() {
        let p = profile("3.5l", 480, 320, false);
        let tree = build(&p, &components(), TemplateKind::Landscape4Col, 0.5).expect("layout");
        assert!(tree.font_px >= 9.0);
    }
}
