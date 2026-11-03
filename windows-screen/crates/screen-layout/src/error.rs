//! 布局错误。

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    NoComponents,
    TooSmall { cell_w: i32, cell_h: i32 },
    UnknownTemplate(String),
    Validation(String),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutError::NoComponents => write!(f, "没有可显示的组件"),
            LayoutError::TooSmall { cell_w, cell_h } => {
                write!(f, "屏幕过小，卡片尺寸 {cell_w}×{cell_h} 低于可读下限")
            }
            LayoutError::UnknownTemplate(name) => write!(f, "未知布局模板：{name}"),
            LayoutError::Validation(msg) => write!(f, "布局校验失败：{msg}"),
        }
    }
}

impl std::error::Error for LayoutError {}
