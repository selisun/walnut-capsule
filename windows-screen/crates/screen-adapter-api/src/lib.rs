//! # screen-adapter-api
//!
//! 设备适配器与传输抽象。只依赖 `screen-core` 的类型，不做 IO。
//! Transport 的占用语义由各实现负责（见 docs/windows-screen/03 §9）。

pub mod error;
pub mod types;

pub use error::AdapterError;
pub use types::{
    public_state_label, DeviceAdapter, DeviceInfo, Health, InitReport, PortCandidate, Transport,
};
