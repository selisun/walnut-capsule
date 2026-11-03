//! 适配器/传输错误。

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterError {
    Busy,
    Disconnected,
    Timeout,
    CapabilityMissing(&'static str),
    Unsupported(&'static str),
    Io(String),
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdapterError::Busy => write!(f, "设备被占用"),
            AdapterError::Disconnected => write!(f, "设备已断开"),
            AdapterError::Timeout => write!(f, "设备操作超时"),
            AdapterError::CapabilityMissing(name) => write!(f, "缺少设备能力：{name}"),
            AdapterError::Unsupported(what) => write!(f, "设备不支持：{what}"),
            AdapterError::Io(msg) => write!(f, "设备 IO 失败：{msg}"),
        }
    }
}

impl std::error::Error for AdapterError {}
