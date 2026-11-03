//! IPC 错误类型。

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcError {
    TooShort { need: usize, got: usize },
    BadMagic,
    UnsupportedMajor { expected: u16, got: u16 },
    UnknownKind(u8),
    PayloadTooLarge { max: usize, got: usize },
    LengthMismatch { header: usize, actual: usize },
    Disconnected,
    Refused(&'static str),
    Json(String),
    Platform(String),
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IpcError::TooShort { need, got } => write!(f, "消息过短：需要 {need}，实际 {got}"),
            IpcError::BadMagic => write!(f, "消息魔数不匹配"),
            IpcError::UnsupportedMajor { expected, got } => {
                write!(f, "IPC 主版本不兼容：期望 {expected}，收到 {got}")
            }
            IpcError::UnknownKind(kind) => write!(f, "未知消息类型：{kind}"),
            IpcError::PayloadTooLarge { max, got } => {
                write!(f, "payload 超限：上限 {max}，实际 {got}")
            }
            IpcError::LengthMismatch { header, actual } => {
                write!(f, "长度字段不符：头部 {header}，实际 {actual}")
            }
            IpcError::Disconnected => write!(f, "IPC 连接已断开"),
            IpcError::Refused(reason) => write!(f, "IPC 拒绝连接：{reason}"),
            IpcError::Json(msg) => write!(f, "payload 解析失败：{msg}"),
            IpcError::Platform(msg) => write!(f, "IPC 平台调用失败：{msg}"),
        }
    }
}

impl std::error::Error for IpcError {}
