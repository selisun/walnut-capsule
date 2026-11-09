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

/// 像素矩形（左上角坐标 + 宽高，`right`/`bottom` 为闭区间）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> u32 {
        self.x.saturating_add(self.width).saturating_sub(1)
    }

    pub fn bottom(&self) -> u32 {
        self.y.saturating_add(self.height).saturating_sub(1)
    }

    pub fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
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
    /// 只刷新帧内一个子矩形（坐标闭区间，取自同一帧的像素）。
    ///
    /// 默认实现返回 [`AdapterError::Unsupported`]，表示该适配器只支持整帧；
    /// 调用方收到 Unsupported 后必须回退到 [`DeviceAdapter::send_frame`] 发全帧，
    /// 不得把设备标记为故障。
    fn send_frame_region(&mut self, _frame: &Frame, _region: Region) -> Result<(), AdapterError> {
        Err(AdapterError::Unsupported("适配器不支持子矩形刷新"))
    }
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

    #[test]
    fn region_uses_closed_interval_bounds() {
        let region = Region::new(20, 200, 80, 40);
        assert_eq!(region.right(), 99);
        assert_eq!(region.bottom(), 239);
        assert_eq!(region.area(), 3_200);
        assert!(!region.is_empty());
        assert!(Region::new(0, 0, 0, 10).is_empty());
    }
}
