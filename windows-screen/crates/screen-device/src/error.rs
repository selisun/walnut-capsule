//! 设备管理层错误。适配器/传输错误在 `screen-adapter-api`。

use std::fmt;

use screen_core::DeviceState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceError {
    MaxDevices(u8),
    AlreadyAttached(String),
    InvalidTransition { from: DeviceState, to: DeviceState },
    NoDevice(usize),
    InvalidConfig(&'static str),
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::MaxDevices(max) => write!(f, "设备数超过上限：{max}"),
            DeviceError::AlreadyAttached(key) => write!(f, "设备已接入：{key}"),
            DeviceError::InvalidTransition { from, to } => {
                write!(f, "非法状态转移：{from:?} -> {to:?}")
            }
            DeviceError::NoDevice(index) => write!(f, "设备索引不存在：{index}"),
            DeviceError::InvalidConfig(name) => write!(f, "非法配置：{name}"),
        }
    }
}

impl std::error::Error for DeviceError {}
