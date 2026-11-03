//! 协议纯函数错误。

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    InvalidDimensions { width: u32, height: u32 },
    BufferLength { expected: usize, actual: usize },
    InvalidChunkSize,
    UnknownCommand(u8),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolError::InvalidDimensions { width, height } => {
                write!(f, "帧尺寸超出协议可表示范围：{width}×{height}")
            }
            ProtocolError::BufferLength { expected, actual } => {
                write!(f, "像素缓冲长度不符：期望 {expected}，实际 {actual}")
            }
            ProtocolError::InvalidChunkSize => write!(f, "分块大小必须大于 0"),
            ProtocolError::UnknownCommand(code) => write!(f, "未验证的命令码：0x{code:02X}"),
        }
    }
}

impl std::error::Error for ProtocolError {}
