//! 布局领域模型。算法在 `screen-layout`。

use serde::{Deserialize, Serialize};

use crate::capabilities::Orientation;
use crate::error::CoreError;
use crate::theme_model::ComponentKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeClass {
    Square,
    Portrait,
    Landscape,
    Wide,
    UltraWide,
}

/// 屏幕逻辑尺寸（不是设备物理尺寸）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenProfile {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub short_edge: u32,
    pub aspect: f32,
    pub orientation: Orientation,
    pub size_class: SizeClass,
    pub is_large: bool,
    pub verified: bool,
}

impl ScreenProfile {
    pub fn new(
        id: impl Into<String>,
        width: u32,
        height: u32,
        orientation: Orientation,
        verified: bool,
    ) -> Result<Self, CoreError> {
        if width == 0 || height == 0 {
            return Err(CoreError::InvalidArgument("screen profile size"));
        }
        let short_edge = width.min(height);
        let aspect = width as f32 / height as f32;
        let size_class = if (0.9..=1.1).contains(&aspect) {
            SizeClass::Square
        } else if aspect < 0.9 {
            SizeClass::Portrait
        } else if aspect <= 2.0 {
            SizeClass::Landscape
        } else if aspect <= 4.0 {
            SizeClass::Wide
        } else {
            SizeClass::UltraWide
        };
        Ok(Self {
            id: id.into(),
            width,
            height,
            short_edge,
            aspect,
            orientation,
            size_class,
            is_large: short_edge >= 600,
            verified,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    pub fn contains_rect(&self, other: &Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    pub fn is_valid(&self) -> bool {
        self.w > 0.0 && self.h > 0.0 && self.x >= 0.0 && self.y >= 0.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutItem {
    pub component: ComponentKind,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutTree {
    pub profile_id: String,
    pub template: String,
    pub font_px: f32,
    pub gap: f32,
    pub items: Vec<LayoutItem>,
}

impl LayoutTree {
    /// 边界与重叠检查；`screen-layout` 在生成后必须调用。
    pub fn validate(&self, width: f32, height: f32) -> Result<(), CoreError> {
        let bounds = Rect::new(0.0, 0.0, width, height);
        for item in &self.items {
            if !item.rect.is_valid() {
                return Err(CoreError::InvalidArgument("layout rect"));
            }
            if !bounds.contains_rect(&item.rect) {
                return Err(CoreError::InvalidArgument("layout out of bounds"));
            }
        }
        for (i, a) in self.items.iter().enumerate() {
            for b in self.items.iter().skip(i + 1) {
                if a.rect.intersects(&b.rect) {
                    return Err(CoreError::InvalidArgument("layout overlap"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_classification() {
        let p = ScreenProfile::new("3.5", 320, 480, Orientation::Portrait, true).expect("profile");
        assert_eq!(p.size_class, SizeClass::Portrait);
        assert_eq!(p.short_edge, 320);

        let p = ScreenProfile::new("3.5-wide", 480, 320, Orientation::Landscape, true)
            .expect("profile");
        assert_eq!(p.size_class, SizeClass::Landscape);

        let p =
            ScreenProfile::new("9.2", 1920, 462, Orientation::Landscape, false).expect("profile");
        assert_eq!(p.size_class, SizeClass::UltraWide);

        let p =
            ScreenProfile::new("2.1", 480, 480, Orientation::Landscape, false).expect("profile");
        assert_eq!(p.size_class, SizeClass::Square);
    }

    #[test]
    fn zero_size_is_rejected() {
        assert!(ScreenProfile::new("x", 0, 480, Orientation::Portrait, false).is_err());
    }

    #[test]
    fn rect_overlap_and_bounds() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(10.0, 0.0, 10.0, 10.0);
        assert!(!a.intersects(&b)); // 仅贴边不算重叠
        let c = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert!(a.intersects(&c));

        let tree = LayoutTree {
            profile_id: "p".into(),
            template: "t".into(),
            font_px: 14.0,
            gap: 8.0,
            items: vec![LayoutItem {
                component: ComponentKind::Cpu,
                rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            }],
        };
        assert!(tree.validate(100.0, 100.0).is_ok());
        assert!(tree.validate(90.0, 100.0).is_err());
    }
}
