//! tiny-skia 画布封装。只暴露矩形/清屏等基础图元，文本与图片在后续阶段接入。

use screen_core::{Color, FrameBuffer, Rect};
use tiny_skia::{FillRule, Paint, Path, Pixmap, Transform};

use crate::error::RenderError;

pub struct Canvas {
    pixmap: Pixmap,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Result<Self, RenderError> {
        let pixmap = Pixmap::new(width, height)
            .ok_or_else(|| RenderError::Canvas(format!("无法分配 {width}×{height} 画布")))?;
        Ok(Self { pixmap })
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    pub fn clear(&mut self, color: Color) {
        self.pixmap.fill(to_color(color));
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) -> Result<(), RenderError> {
        let r = tiny_skia::Rect::from_xywh(rect.x, rect.y, rect.w, rect.h)
            .ok_or_else(|| RenderError::Canvas("矩形参数非法".to_string()))?;
        let mut paint = Paint::default();
        paint.set_color(to_color(color));
        paint.anti_alias = false;
        self.pixmap
            .fill_rect(r, &paint, Transform::identity(), None);
        Ok(())
    }

    pub fn fill_path(&mut self, path: &Path, color: Color) -> Result<(), RenderError> {
        let mut paint = Paint::default();
        paint.set_color(to_color(color));
        paint.anti_alias = true;
        self.pixmap
            .fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
        Ok(())
    }

    pub fn into_frame(self) -> Result<FrameBuffer, RenderError> {
        let width = self.pixmap.width();
        let height = self.pixmap.height();
        let pixels = self.pixmap.data().to_vec();
        FrameBuffer::new(width, height, pixels)
            .map_err(|e| RenderError::InvalidLayout(e.to_string()))
    }
}

fn to_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clears_canvas_to_color() {
        let mut canvas = Canvas::new(4, 3).expect("canvas");
        canvas.clear(Color::rgb(10, 20, 30));
        let frame = canvas.into_frame().expect("frame");
        assert_eq!(frame.width(), 4);
        assert_eq!(frame.height(), 3);
        assert_eq!(&frame.pixels()[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn fills_rect() {
        let mut canvas = Canvas::new(8, 8).expect("canvas");
        canvas.clear(Color::rgb(0, 0, 0));
        canvas
            .fill_rect(Rect::new(2.0, 2.0, 4.0, 4.0), Color::rgb(255, 0, 0))
            .expect("fill");
        let frame = canvas.into_frame().expect("frame");
        let idx = (2 * 8 + 2) * 4;
        assert_eq!(&frame.pixels()[idx..idx + 4], &[255, 0, 0, 255]);
        let idx = 0;
        assert_eq!(&frame.pixels()[idx..idx + 4], &[0, 0, 0, 255]);
    }

    #[test]
    fn zero_size_canvas_fails() {
        assert!(Canvas::new(0, 10).is_err());
    }
}
