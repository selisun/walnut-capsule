//! 用户方向设置到逻辑 Profile / 布局模板 / 最终旋转的纯函数映射。
//!
//! 设备物理帧始终由适配器能力决定；本模块只决定“逻辑画布如何旋转到物理画布”。

use std::fmt;

use screen_core::{FrameBuffer, Orientation, Rotation, ScreenProfile};
use screen_layout::TemplateKind;
use screen_render::orient;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectionError {
    InvalidOrientation(String),
    InvalidFlip(String),
    InvalidPhysical(String),
    InvalidLogical(String),
}

impl fmt::Display for DirectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DirectionError::InvalidOrientation(v) => write!(f, "display.orientation 非法：{v}"),
            DirectionError::InvalidFlip(v) => write!(f, "display.flip 非法：{v}"),
            DirectionError::InvalidPhysical(v) => write!(f, "物理 Profile 非法：{v}"),
            DirectionError::InvalidLogical(v) => write!(f, "逻辑帧尺寸非法：{v}"),
        }
    }
}

impl std::error::Error for DirectionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOrientation {
    Portrait,
    Landscape,
}

impl LogicalOrientation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::Landscape => "landscape",
        }
    }

    fn from_orientation(value: Orientation) -> Self {
        match value {
            Orientation::Portrait => Self::Portrait,
            Orientation::Landscape => Self::Landscape,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flip {
    Normal,
    Flipped,
}

impl Flip {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Flipped => "flipped",
        }
    }
}

/// 两个独立维度：屏幕方向 + 方向对调。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Direction {
    pub orientation: LogicalOrientation,
    pub flip: Flip,
}

impl Direction {
    /// 解析配置值。`auto` 解析为设备原生方向；3.5 原生 Portrait，因此 auto=portrait。
    pub fn parse(
        orientation: &str,
        flip: &str,
        physical_orientation: Orientation,
    ) -> Result<Self, DirectionError> {
        let orientation = match orientation {
            "portrait" => LogicalOrientation::Portrait,
            "landscape" => LogicalOrientation::Landscape,
            "auto" | "" => LogicalOrientation::from_orientation(physical_orientation),
            other => return Err(DirectionError::InvalidOrientation(other.to_string())),
        };
        let flip = match flip {
            "normal" | "" => Flip::Normal,
            "flipped" => Flip::Flipped,
            other => return Err(DirectionError::InvalidFlip(other.to_string())),
        };
        Ok(Self { orientation, flip })
    }

    /// 最终方向：与物理原生方向一致为 0°，否则 90°；flip 追加 180°。
    pub fn rotation(self, physical_orientation: Orientation) -> Rotation {
        let same = matches!(
            (self.orientation, physical_orientation),
            (LogicalOrientation::Portrait, Orientation::Portrait)
                | (LogicalOrientation::Landscape, Orientation::Landscape)
        );
        let base = if same {
            Rotation::None
        } else {
            Rotation::Deg90
        };
        if self.flip == Flip::Normal {
            base
        } else {
            match base {
                Rotation::None => Rotation::Deg180,
                Rotation::Deg90 => Rotation::Deg270,
                other => other,
            }
        }
    }

    /// 逻辑 Profile：肖像取短边×长边；横屏取长边×短边。flip 不改变宽高。
    pub fn logical_profile(
        self,
        physical: &ScreenProfile,
    ) -> Result<ScreenProfile, DirectionError> {
        let long = physical.width.max(physical.height);
        let short = physical.width.min(physical.height);
        let (width, height, orientation) = match self.orientation {
            LogicalOrientation::Portrait => (short, long, Orientation::Portrait),
            LogicalOrientation::Landscape => (long, short, Orientation::Landscape),
        };
        let id = format!("{}-{}", physical.id, self.orientation.as_str());
        ScreenProfile::new(id, width, height, orientation, physical.verified)
            .map_err(|e| DirectionError::InvalidPhysical(e.to_string()))
    }

    pub fn plan(
        self,
        physical: &ScreenProfile,
        template_override: Option<&str>,
    ) -> Result<FramePlan, DirectionError> {
        if physical.width == 0 || physical.height == 0 {
            return Err(DirectionError::InvalidPhysical("零尺寸".to_string()));
        }
        let logical = self.logical_profile(physical)?;
        let template = template_override
            .and_then(TemplateKind::from_override)
            .unwrap_or_else(|| TemplateKind::for_profile(&logical));
        Ok(FramePlan {
            physical: physical.clone(),
            logical,
            template,
            rotation: self.rotation(physical.orientation),
        })
    }

    /// 把逻辑 RGBA 帧旋转为物理 RGBA 帧；物理尺寸必须与 Profile 完全一致。
    pub fn oriented_frame(
        self,
        physical: &ScreenProfile,
        logical: &FrameBuffer,
    ) -> Result<FrameBuffer, DirectionError> {
        let plan = self.plan(physical, None)?;
        if (logical.width(), logical.height()) != (plan.logical.width, plan.logical.height) {
            return Err(DirectionError::InvalidLogical(format!(
                "{}x{}，预期 {}x{}",
                logical.width(),
                logical.height(),
                plan.logical.width,
                plan.logical.height
            )));
        }
        let out = orient(logical, plan.rotation);
        if (out.width(), out.height()) != (physical.width, physical.height) {
            return Err(DirectionError::InvalidPhysical(format!(
                "旋转后 {}x{}，预期 {}x{}",
                out.width(),
                out.height(),
                physical.width,
                physical.height
            )));
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FramePlan {
    pub physical: ScreenProfile,
    pub logical: ScreenProfile,
    pub template: TemplateKind,
    pub rotation: Rotation,
}

#[cfg(test)]
mod tests {
    use super::*;
    use screen_core::Orientation;

    fn physical() -> ScreenProfile {
        ScreenProfile::new("3.5-physical", 320, 480, Orientation::Portrait, true).expect("physical")
    }

    fn unique_frame(w: u32, h: u32) -> FrameBuffer {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&[
                    (x & 0xFF) as u8,
                    (y & 0xFF) as u8,
                    ((((x >> 8) & 1) | (((y >> 8) & 1) << 1)) & 0xFF) as u8,
                    255,
                ]);
            }
        }
        FrameBuffer::new(w, h, px).expect("frame")
    }

    fn pixel(frame: &FrameBuffer, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * frame.width() + x) * 4) as usize;
        [
            frame.pixels()[i],
            frame.pixels()[i + 1],
            frame.pixels()[i + 2],
            frame.pixels()[i + 3],
        ]
    }

    fn mapped_source_xy(
        rotation: Rotation,
        src_w: u32,
        src_h: u32,
        dx: u32,
        dy: u32,
    ) -> (u32, u32) {
        match rotation {
            Rotation::None => (dx, dy),
            Rotation::Deg180 => (src_w - 1 - dx, src_h - 1 - dy),
            Rotation::Deg90 => (dy, src_h - 1 - dx),
            Rotation::Deg270 => (src_w - 1 - dy, dx),
        }
    }

    fn source_to_dest_xy(
        rotation: Rotation,
        src_w: u32,
        src_h: u32,
        sx: u32,
        sy: u32,
    ) -> (u32, u32) {
        match rotation {
            Rotation::None => (sx, sy),
            Rotation::Deg180 => (src_w - 1 - sx, src_h - 1 - sy),
            Rotation::Deg90 => (src_h - 1 - sy, sx),
            Rotation::Deg270 => (sy, src_w - 1 - sx),
        }
    }

    fn fill_rect(buf: &mut [u8], w: u32, x0: u32, y0: u32, ww: u32, hh: u32, color: [u8; 4]) {
        for y in y0..y0 + hh {
            for x in x0..x0 + ww {
                let i = ((y * w + x) * 4) as usize;
                buf[i..i + 4].copy_from_slice(&color);
            }
        }
    }

    fn marker_fixture(w: u32, h: u32) -> FrameBuffer {
        let mut px = vec![0u8; (w * h * 4) as usize];
        // 四色角标
        fill_rect(&mut px, w, 0, 0, 32, 32, [255, 0, 0, 255]); // TL 红
        fill_rect(&mut px, w, w - 32, 0, 32, 32, [0, 255, 0, 255]); // TR 绿
        fill_rect(&mut px, w, 0, h - 32, 32, 32, [0, 0, 255, 255]); // BL 蓝
        fill_rect(&mut px, w, w - 32, h - 32, 32, 32, [255, 255, 255, 255]); // BR 白
                                                                             // 边界：4px 黄
        fill_rect(&mut px, w, 0, 0, w, 4, [255, 255, 0, 255]);
        fill_rect(&mut px, w, 0, h - 4, w, 4, [255, 255, 0, 255]);
        fill_rect(&mut px, w, 0, 0, 4, h, [255, 255, 0, 255]);
        fill_rect(&mut px, w, w - 4, 0, 4, h, [255, 255, 0, 255]);
        // 中心十字
        fill_rect(
            &mut px,
            w,
            w / 2 - 2,
            h / 2 - 40,
            4,
            80,
            [255, 255, 255, 255],
        );
        fill_rect(
            &mut px,
            w,
            w / 2 - 40,
            h / 2 - 2,
            80,
            4,
            [255, 255, 255, 255],
        );
        // 文字/图标代表块（测试方向，不涉及真实字形）
        fill_rect(&mut px, w, 48, 48, 80, 16, [10, 20, 30, 255]);
        fill_rect(&mut px, w, w - 64, 48, 16, 16, [40, 50, 60, 255]);
        FrameBuffer::new(w, h, px).expect("fixture")
    }

    fn corner_only_fixture(w: u32, h: u32) -> FrameBuffer {
        let mut px = vec![0u8; (w * h * 4) as usize];
        fill_rect(&mut px, w, 0, 0, 32, 32, [255, 0, 0, 255]);
        fill_rect(&mut px, w, w - 32, 0, 32, 32, [0, 255, 0, 255]);
        fill_rect(&mut px, w, 0, h - 32, 32, 32, [0, 0, 255, 255]);
        fill_rect(&mut px, w, w - 32, h - 32, 32, 32, [255, 255, 255, 255]);
        FrameBuffer::new(w, h, px).expect("fixture")
    }

    #[test]
    fn four_direction_combinations_resolve_to_expected_profiles_and_rotations() {
        let physical = physical();
        let cases = [
            (
                "portrait",
                "normal",
                320,
                480,
                Orientation::Portrait,
                Rotation::None,
            ),
            (
                "portrait",
                "flipped",
                320,
                480,
                Orientation::Portrait,
                Rotation::Deg180,
            ),
            (
                "landscape",
                "normal",
                480,
                320,
                Orientation::Landscape,
                Rotation::Deg90,
            ),
            (
                "landscape",
                "flipped",
                480,
                320,
                Orientation::Landscape,
                Rotation::Deg270,
            ),
        ];
        for (orientation, flip, width, height, expected_orientation, rotation) in cases {
            let direction =
                Direction::parse(orientation, flip, physical.orientation).expect("direction");
            let plan = direction.plan(&physical, None).expect("plan");
            assert_eq!((plan.logical.width, plan.logical.height), (width, height));
            assert_eq!(plan.logical.orientation, expected_orientation);
            assert_eq!(plan.rotation, rotation);
            assert_eq!(plan.physical.width, 320);
            assert_eq!(plan.physical.height, 480);
        }
    }

    #[test]
    fn auto_defaults_to_device_native_orientation() {
        let direction =
            Direction::parse("auto", "normal", Orientation::Portrait).expect("direction");
        assert_eq!(direction.orientation, LogicalOrientation::Portrait);
    }

    #[test]
    fn invalid_values_are_rejected() {
        assert!(Direction::parse("sideways", "normal", Orientation::Portrait).is_err());
        assert!(Direction::parse("portrait", "mirror", Orientation::Portrait).is_err());
    }

    #[test]
    fn flip_does_not_swap_width_height() {
        let physical = physical();
        let normal = Direction::parse("portrait", "normal", Orientation::Portrait).expect("n");
        let flipped = Direction::parse("portrait", "flipped", Orientation::Portrait).expect("f");
        let a = normal.logical_profile(&physical).expect("a");
        let b = flipped.logical_profile(&physical).expect("b");
        assert_eq!((a.width, a.height), (b.width, b.height));
        assert_eq!((b.width, b.height), (320, 480));
    }

    #[test]
    fn landscape_uses_landscape_layout_not_portrait_template() {
        let physical = physical();
        let direction =
            Direction::parse("landscape", "normal", Orientation::Portrait).expect("direction");
        let plan = direction.plan(&physical, None).expect("plan");
        assert_eq!(plan.template, TemplateKind::Landscape4Col);
        assert_eq!((plan.logical.width, plan.logical.height), (480, 320));
    }

    #[test]
    fn four_modes_map_all_153600_pixels_to_320x480() {
        let physical = physical();
        let cases = [
            ("portrait", "normal", 320, 480, Rotation::None),
            ("portrait", "flipped", 320, 480, Rotation::Deg180),
            ("landscape", "normal", 480, 320, Rotation::Deg90),
            ("landscape", "flipped", 480, 320, Rotation::Deg270),
        ];
        for (orientation, flip, src_w, src_h, rotation) in cases {
            let direction =
                Direction::parse(orientation, flip, physical.orientation).expect("direction");
            let src = unique_frame(src_w, src_h);
            let out = direction.oriented_frame(&physical, &src).expect("oriented");
            assert_eq!((out.width(), out.height()), (320, 480));
            assert_eq!(out.pixels().len(), 320 * 480 * 4);
            for dy in 0..480 {
                for dx in 0..320 {
                    let (sx, sy) = mapped_source_xy(rotation, src_w, src_h, dx, dy);
                    assert_eq!(pixel(&out, dx, dy), pixel(&src, sx, sy));
                }
            }
        }
    }

    #[test]
    fn four_modes_map_borderless_corner_pixels_exactly() {
        let physical = physical();
        let cases = [
            (
                "portrait",
                "normal",
                320,
                480,
                [
                    [255, 0, 0, 255],
                    [0, 255, 0, 255],
                    [0, 0, 255, 255],
                    [255, 255, 255, 255],
                ],
            ),
            (
                "portrait",
                "flipped",
                320,
                480,
                [
                    [255, 255, 255, 255],
                    [0, 0, 255, 255],
                    [0, 255, 0, 255],
                    [255, 0, 0, 255],
                ],
            ),
            (
                "landscape",
                "normal",
                480,
                320,
                [
                    [0, 0, 255, 255],
                    [255, 0, 0, 255],
                    [255, 255, 255, 255],
                    [0, 255, 0, 255],
                ],
            ),
            (
                "landscape",
                "flipped",
                480,
                320,
                [
                    [0, 255, 0, 255],
                    [255, 255, 255, 255],
                    [255, 0, 0, 255],
                    [0, 0, 255, 255],
                ],
            ),
        ];
        for (orientation, flip, w, h, expected) in cases {
            let direction =
                Direction::parse(orientation, flip, physical.orientation).expect("direction");
            let src = corner_only_fixture(w, h);
            let out = direction.oriented_frame(&physical, &src).expect("oriented");
            assert_eq!(pixel(&out, 0, 0), expected[0], "{orientation}/{flip} TL");
            assert_eq!(pixel(&out, 319, 0), expected[1], "{orientation}/{flip} TR");
            assert_eq!(pixel(&out, 0, 479), expected[2], "{orientation}/{flip} BL");
            assert_eq!(
                pixel(&out, 319, 479),
                expected[3],
                "{orientation}/{flip} BR"
            );
        }
    }

    #[test]
    fn four_modes_preserve_corners_border_cross_and_text_icon_regions() {
        let physical = physical();
        let cases = [
            ("portrait", "normal", 320, 480, Rotation::None),
            ("portrait", "flipped", 320, 480, Rotation::Deg180),
            ("landscape", "normal", 480, 320, Rotation::Deg90),
            ("landscape", "flipped", 480, 320, Rotation::Deg270),
        ];
        for (orientation, flip, src_w, src_h, rotation) in cases {
            let direction =
                Direction::parse(orientation, flip, physical.orientation).expect("direction");
            let src = marker_fixture(src_w, src_h);
            let out = direction.oriented_frame(&physical, &src).expect("oriented");

            // 角标中心（避开 4px 黄色边界）
            let corner_samples = [
                (16, 16, [255, 0, 0, 255]),
                (src_w - 16, 16, [0, 255, 0, 255]),
                (16, src_h - 16, [0, 0, 255, 255]),
                (src_w - 16, src_h - 16, [255, 255, 255, 255]),
            ];
            for (sx, sy, color) in corner_samples {
                let (dx, dy) = source_to_dest_xy(rotation, src_w, src_h, sx, sy);
                assert_eq!(pixel(&out, dx, dy), color, "corner src({sx},{sy})");
            }

            // 物理四角被黄色边界覆盖
            assert_eq!(pixel(&out, 0, 0), [255, 255, 0, 255], "border TL");
            assert_eq!(pixel(&out, 319, 0), [255, 255, 0, 255], "border TR");
            assert_eq!(pixel(&out, 0, 479), [255, 255, 0, 255], "border BL");
            assert_eq!(pixel(&out, 319, 479), [255, 255, 0, 255], "border BR");

            // 中心十字
            let (cx, cy) = source_to_dest_xy(rotation, src_w, src_h, src_w / 2, src_h / 2);
            assert_eq!(pixel(&out, cx, cy), [255, 255, 255, 255], "center cross");

            // 文字/图标代表块的方向：映射后的左上点仍为对应颜色。
            let (text_x, text_y) = source_to_dest_xy(rotation, src_w, src_h, 48, 48);
            let (icon_x, icon_y) = source_to_dest_xy(rotation, src_w, src_h, src_w - 64, 48);
            assert_eq!(
                pixel(&out, text_x, text_y),
                [10, 20, 30, 255],
                "text marker"
            );
            assert_eq!(
                pixel(&out, icon_x, icon_y),
                [40, 50, 60, 255],
                "icon marker"
            );
        }
    }
}
