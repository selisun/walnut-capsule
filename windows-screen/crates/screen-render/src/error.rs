//! 渲染错误。

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    Canvas(String),
    InvalidLayout(String),
    TextUnavailable,
    Text(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::Canvas(msg) => write!(f, "画布错误：{msg}"),
            RenderError::InvalidLayout(msg) => write!(f, "布局不可渲染：{msg}"),
            RenderError::TextUnavailable => write!(f, "文本渲染尚未实现"),
            RenderError::Text(msg) => write!(f, "文本渲染失败：{msg}"),
        }
    }
}

impl std::error::Error for RenderError {}
