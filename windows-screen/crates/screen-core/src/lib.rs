//! # screen-core
//!
//! 核心领域模型：设备能力、设备状态、数据快照、主题数据、布局数据、帧缓冲。
//!
//! 明确不包含：
//! - NAS 地址 / Token / 凭据
//! - 设备 IO / 串口 / USB
//! - 具体渲染实现

pub mod capabilities;
pub mod device_state;
pub mod error;
pub mod frame;
pub mod layout_model;
pub mod snapshot;
pub mod theme_model;

pub use capabilities::{
    BrightnessRange, Cap, DeviceCapabilities, FrameSpec, Orientation, PixelFormat, RefreshMode,
    Rotation, Source, TransportKind,
};
pub use device_state::{can_transition, DeviceState};
pub use error::CoreError;
pub use frame::{bytes_per_pixel, Frame, FrameBuffer};
pub use layout_model::{LayoutItem, LayoutTree, Rect, ScreenProfile, SizeClass};
pub use snapshot::{
    CpuMetrics, DiskMetrics, GpuMetrics, MemoryMetrics, Metrics, NasInfo, NetworkMetrics,
    PcMetrics, Snapshot, StorageVolume,
};
pub use theme_model::{
    Background, Color, ComponentKind, ComponentStyle, Components, ImageAnchor, ImageFit,
    LayoutConfig, Palette, ResourceKind, ResourceRef, ThemeConfig, Typography,
};
