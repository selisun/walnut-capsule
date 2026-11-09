//! 文本渲染接口。本阶段没有字体栈/排版实现，避免引入未批准的字体依赖。

use screen_core::Color;

use crate::canvas::Canvas;
use crate::error::RenderError;

#[derive(Debug, Clone, PartialEq)]
pub struct FontSpec {
    pub family: String,
    pub size_px: f32,
    pub weight: u16,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            family: "Microsoft YaHei".to_string(),
            size_px: 14.0,
            weight: 400,
        }
    }
}

pub trait TextRenderer {
    fn draw_text(
        &mut self,
        canvas: &mut Canvas,
        text: &str,
        x: f32,
        y: f32,
        max_width: f32,
        size: f32,
        color: Color,
    ) -> Result<(), RenderError>;

    /// 量宽（像素）。默认实现返回 None，调用方退回按字符数估算。
    fn measure_text(&mut self, _text: &str, _size: f32) -> Option<f32> {
        None
    }
}

/// 占位实现：明确返回“未实现”，不伪造文字。
pub struct UnavailableTextRenderer;

impl TextRenderer for UnavailableTextRenderer {
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

/// 按平台创建文字渲染器；非 Windows 明确降级为不可用。
pub fn create_text_renderer(spec: &FontSpec) -> Result<Box<dyn TextRenderer>, RenderError> {
    #[cfg(windows)]
    {
        crate::font::GdiTextRenderer::new(spec).map(|r| Box::new(r) as Box<dyn TextRenderer>)
    }
    #[cfg(not(windows))]
    {
        let _ = spec;
        Err(RenderError::TextUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_renderer_still_returns_text_unavailable() {
        let mut canvas = Canvas::new(32, 16).expect("canvas");
        let mut renderer = UnavailableTextRenderer;
        let err = renderer
            .draw_text(
                &mut canvas,
                "CPU 42%",
                0.0,
                0.0,
                32.0,
                12.0,
                Color::rgb(255, 255, 255),
            )
            .unwrap_err();
        assert_eq!(err, RenderError::TextUnavailable);
    }

    #[cfg(windows)]
    #[test]
    fn factory_rejects_non_positive_size() {
        let spec = FontSpec {
            size_px: 0.0,
            ..FontSpec::default()
        };
        match create_text_renderer(&spec) {
            Ok(_) => panic!("字号 0 应被拒绝"),
            Err(RenderError::Text(msg)) => assert!(msg.contains("字号")),
            Err(other) => panic!("意外的错误类型：{other:?}"),
        }

        let spec = FontSpec {
            size_px: f32::NAN,
            ..FontSpec::default()
        };
        assert!(create_text_renderer(&spec).is_err());
    }
}
