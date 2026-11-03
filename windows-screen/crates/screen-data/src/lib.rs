//! # screen-data
//!
//! 只消费主客户端经 IPC 推送的快照：
//! 序列号、时间戳、过期判定、`Ready/Stale/WaitingClient/VersionMismatch` 状态。
//!
//! 明确禁止：自行连接 NAS、读取/保存 Token、实现第二套采集。

pub mod error;
pub mod gate;
pub mod state;

pub use error::DataError;
pub use gate::SnapshotGate;
pub use state::{effective_threshold_ms, DataState, DataStatus};
