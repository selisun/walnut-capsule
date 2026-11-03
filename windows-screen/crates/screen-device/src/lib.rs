//! # screen-device
//!
//! 设备层：设备生命周期与状态机。适配器/传输抽象在 `screen-adapter-api`。
//!
//! 本阶段**不打开任何真实设备**；打开/通信由 `screen-transport-cdc` 与型号适配器实现。
//! 每个 Transport 的占用检测必须在自身实现中完成（`03` §9）；禁止为取得控制权终止其他进程。

pub mod error;
pub mod manager;

pub use error::DeviceError;
pub use manager::{DeviceManager, ManagedDevice};
