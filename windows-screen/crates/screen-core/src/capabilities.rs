//! 设备能力模型。核心规则：未知能力不得用默认值冒充。

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// 三态能力：已知 / 未知 / 明确不支持。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "value", rename_all = "snake_case")]
pub enum Cap<T> {
    Known(T),
    Unknown,
    Unsupported,
}

impl<T> Cap<T> {
    pub fn is_known(&self) -> bool {
        matches!(self, Cap::Known(_))
    }

    pub fn known(&self) -> Option<&T> {
        match self {
            Cap::Known(v) => Some(v),
            _ => None,
        }
    }

    pub fn is_unsupported(&self) -> bool {
        matches!(self, Cap::Unsupported)
    }
}

impl<T: Copy> Cap<T> {
    pub fn copied(&self) -> Option<T> {
        self.known().copied()
    }
}

/// 通信方式。`Unknown` 用于尚未识别的 Transport。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Cdc,
    Hid,
    VendorDriver,
    VendorSdk,
    Native,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Landscape,
    Portrait,
}

impl Orientation {
    pub fn is_portrait(self) -> bool {
        matches!(self, Orientation::Portrait)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    None,
    Deg90,
    Deg180,
    Deg270,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    Rgb565Le,
    Rgb565Be,
    Rgb888,
    Mono,
    /// 内部渲染缓冲格式（不直接下发给设备）。
    Rgba8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshMode {
    Full,
    Partial,
    NeedsAck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrightnessRange {
    pub min: u8,
    pub max: u8,
}

impl BrightnessRange {
    pub fn validate(self) -> Result<Self, CoreError> {
        if self.min > self.max {
            return Err(CoreError::InvalidArgument(
                "brightness.min > brightness.max",
            ));
        }
        Ok(self)
    }
}

/// 结论来源，与 `03` 证据表口径一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    OfficialAnalyzed,
    OfficialDoc,
    LegacyVerified,
    CombinationMachineVerified,
    MachineVerified,
    Inferred,
    Unknown,
}

/// 设备能力集合。字段缺失一律显式 `Unknown`/`Unsupported`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceCapabilities {
    pub model_id: String,
    pub transport: TransportKind,
    pub frame_width: Cap<u32>,
    pub frame_height: Cap<u32>,
    pub native_orientation: Cap<Orientation>,
    pub rotation_modes: Cap<Vec<Rotation>>,
    pub mirror: Cap<bool>,
    pub invert: Cap<bool>,
    pub pixel_format: Cap<PixelFormat>,
    pub row_padding: Cap<u32>,
    pub max_payload: Cap<usize>,
    pub chunk_size: Cap<usize>,
    pub refresh_mode: Cap<RefreshMode>,
    pub supports_readback: Cap<bool>,
    pub brightness: Cap<BrightnessRange>,
    pub verified: bool,
    pub source: Source,
}

impl DeviceCapabilities {
    /// 只构造已登记型号的基础能力；其余能力默认 Unknown。
    pub fn registered(
        model_id: impl Into<String>,
        transport: TransportKind,
        verified: bool,
        source: Source,
    ) -> Self {
        Self {
            model_id: model_id.into(),
            transport,
            frame_width: Cap::Unknown,
            frame_height: Cap::Unknown,
            native_orientation: Cap::Unknown,
            rotation_modes: Cap::Unknown,
            mirror: Cap::Unknown,
            invert: Cap::Unknown,
            pixel_format: Cap::Unknown,
            row_padding: Cap::Unknown,
            max_payload: Cap::Unknown,
            chunk_size: Cap::Unknown,
            refresh_mode: Cap::Unknown,
            supports_readback: Cap::Unknown,
            brightness: Cap::Unknown,
            verified,
            source,
        }
    }

    /// 产出帧所需的最小能力集。任一缺失都返回 `MissingCapability`，不得默认。
    pub fn frame_spec(&self) -> Result<FrameSpec, CoreError> {
        let width = *self
            .frame_width
            .known()
            .ok_or(CoreError::MissingCapability("frame_width"))?;
        let height = *self
            .frame_height
            .known()
            .ok_or(CoreError::MissingCapability("frame_height"))?;
        let pixel_format = *self
            .pixel_format
            .known()
            .ok_or(CoreError::MissingCapability("pixel_format"))?;
        if width == 0 || height == 0 {
            return Err(CoreError::InvalidArgument("frame size"));
        }
        let max_payload = self.max_payload.copied();
        if max_payload == Some(0) {
            return Err(CoreError::InvalidArgument("max_payload"));
        }
        Ok(FrameSpec {
            width,
            height,
            pixel_format,
            row_padding: self.row_padding.copied().unwrap_or(0),
            max_payload,
            chunk_hint: self.chunk_size.copied(),
        })
    }

    pub fn orientation(&self) -> Option<Orientation> {
        self.native_orientation.copied()
    }
}

/// 帧编码参数（不含具体协议字节）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSpec {
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    pub row_padding: u32,
    pub max_payload: Option<usize>,
    pub chunk_hint: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_capabilities_must_fail_instead_of_defaulting() {
        let caps = DeviceCapabilities::registered(
            "turzx-3.5-cdc",
            TransportKind::Cdc,
            true,
            Source::LegacyVerified,
        );
        assert!(caps.frame_spec().is_err());
        assert_eq!(caps.orientation(), None);
    }

    #[test]
    fn known_capabilities_produce_frame_spec() {
        let mut caps =
            DeviceCapabilities::registered("demo", TransportKind::Cdc, false, Source::Unknown);
        caps.frame_width = Cap::Known(480);
        caps.frame_height = Cap::Known(320);
        caps.pixel_format = Cap::Known(PixelFormat::Rgb565Le);
        caps.chunk_size = Cap::Known(8192);
        let spec = caps.frame_spec().expect("frame spec");
        assert_eq!((spec.width, spec.height), (480, 320));
        assert_eq!(spec.chunk_hint, Some(8192));
    }

    #[test]
    fn unverified_is_preserved() {
        let caps = DeviceCapabilities::registered(
            "placeholder",
            TransportKind::Unknown,
            false,
            Source::Inferred,
        );
        assert!(!caps.verified);
        assert_eq!(caps.source, Source::Inferred);
    }
}
