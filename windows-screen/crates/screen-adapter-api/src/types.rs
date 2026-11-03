//! 传输/适配器的公共类型。

use screen_core::{DeviceCapabilities, Frame, TransportKind};

use crate::error::AdapterError;

/// 枚举到的候选设备。`open_path` 来自同一 SetupDi 元素的接口路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortCandidate {
    pub key: String,
    pub description: String,
    pub transport: TransportKind,
    /// PnP 实例后缀（**不是已证实的 USB 序列号**）。
    pub stable_id: Option<String>,
    /// 打开用的设备接口路径（`\\?\...`），与 `key` 来自同一元素。
    pub open_path: Option<String>,
}

/// 已识别并准备打开的设备。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub key: String,
    pub model_id: String,
    pub stable_id: Option<String>,
    pub transport: TransportKind,
    pub open_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    pub responsive: bool,
    pub last_error: Option<String>,
}

impl Health {
    pub fn ok() -> Self {
        Self {
            responsive: true,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InitReport {
    pub firmware: Option<String>,
    pub notes: Vec<String>,
}

pub trait Transport {
    fn kind(&self) -> TransportKind;
    fn enumerate(&mut self) -> Result<Vec<PortCandidate>, AdapterError>;
    fn open(&mut self, candidate: &PortCandidate) -> Result<(), AdapterError>;
    fn write(&mut self, bytes: &[u8], timeout_ms: u32) -> Result<usize, AdapterError>;
    fn read(&mut self, buf: &mut [u8], timeout_ms: u32) -> Result<usize, AdapterError>;
    fn close(&mut self);
}

pub trait DeviceAdapter {
    fn model_id(&self) -> &str;
    fn capabilities(&self) -> &DeviceCapabilities;
    fn open(&mut self, device: &DeviceInfo) -> Result<(), AdapterError>;
    fn init(&mut self) -> Result<InitReport, AdapterError>;
    fn send_frame(&mut self, frame: &Frame) -> Result<(), AdapterError>;
    fn set_brightness(&mut self, level: u8) -> Result<(), AdapterError>;
    fn poll_health(&mut self) -> Result<Health, AdapterError>;
    fn close(&mut self);
}

pub fn public_state_label(state: screen_core::DeviceState) -> &'static str {
    use screen_core::DeviceState::*;
    match state {
        Discovered => "discovered",
        Identified => "identified",
        Opening => "opening",
        Initializing => "initializing",
        Ready => "ready",
        Streaming => "streaming",
        Degraded => "degraded",
        Disconnected => "disconnected",
        Busy => "busy",
        Error => "error",
        Unsupported => "unsupported",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use screen_core::DeviceState;

    #[test]
    fn state_labels_are_stable() {
        assert_eq!(public_state_label(DeviceState::Busy), "busy");
        assert_eq!(public_state_label(DeviceState::Ready), "ready");
    }
}
