//! # screen-ipc
//!
//! 本地 IPC 协议层：消息头、长度校验、版本协商、消息类型、连接抽象，以及 Windows 命名管道服务端。
//!
//! 明确不包含：
//! - NAS Token / 凭据（协议中不存在此类字段）
//! - 任何出站网络连接

pub mod error;
pub mod header;
pub mod message;
pub mod peer;
pub mod transport;

#[cfg(windows)]
pub mod windows_pipe;

pub use error::IpcError;
pub use header::{
    Header, MessageKind, Version, HEADER_LEN, MAGIC, MAX_FRAME_BYTES, MAX_PAYLOAD_BYTES,
    SCHEMA_MAJOR, SCHEMA_MINOR,
};
pub use message::Message;
pub use peer::ImageNameVerifier;
pub use transport::{ConnectionState, IpcClient, IpcServer, PeerInfo, PeerVerifier};

#[cfg(windows)]
pub use windows_pipe::{normalize_pipe_name, WindowsPipeServer, DEFAULT_PIPE_NAME};
