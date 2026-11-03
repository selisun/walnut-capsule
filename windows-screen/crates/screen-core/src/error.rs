//! 核心领域错误。各模块在自己的错误类型中包装本类型。

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    InvalidArgument(&'static str),
    MissingCapability(&'static str),
    Unsupported(&'static str),
    BufferLength { expected: usize, actual: usize },
    NumericOverflow { field: &'static str },
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::InvalidArgument(name) => write!(f, "非法参数：{name}"),
            CoreError::MissingCapability(name) => write!(f, "设备能力未知：{name}"),
            CoreError::Unsupported(what) => write!(f, "不支持：{what}"),
            CoreError::BufferLength { expected, actual } => {
                write!(f, "缓冲区长度不符：期望 {expected}，实际 {actual}")
            }
            CoreError::NumericOverflow { field } => write!(f, "数值溢出：{field}"),
        }
    }
}

impl std::error::Error for CoreError {}
