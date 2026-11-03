//! IPC 传输抽象与对端身份接口。
//!
//! 本阶段**不实现** Windows 命名管道。实现阶段必须按 `09` §11：
//! DACL 仅当前用户、拒绝远程、`GetNamedPipeClientProcessId` 校验 PID/SID/映像，
//! nonce 只用于防重放，不作为身份依据。

use crate::error::IpcError;
use crate::message::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    WaitingClient,
    VersionMismatch,
}

impl ConnectionState {
    pub fn is_connected(self) -> bool {
        matches!(self, ConnectionState::Connected)
    }
}

/// 对端身份材料；由平台层填充（本阶段仅抽象）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerInfo {
    pub process_id: u32,
    pub user_sid: Option<String>,
    pub image_path: Option<String>,
    pub image_sha256: Option<String>,
}

/// 对端校验器。实现必须同时校验 SID、映像路径与哈希/签名，不能只看 nonce。
pub trait PeerVerifier {
    fn verify(&self, peer: &PeerInfo) -> Result<(), IpcError>;
}

pub trait IpcClient {
    fn connect(&mut self) -> Result<(), IpcError>;
    fn send(&mut self, message: &Message) -> Result<(), IpcError>;
    fn recv(&mut self) -> Result<Message, IpcError>;
    fn state(&self) -> ConnectionState;
    fn close(&mut self);
}

pub trait IpcServer {
    /// 接受一个已通过平台身份校验的客户端。
    fn accept(&mut self, verifier: &dyn PeerVerifier) -> Result<Box<dyn IpcClient>, IpcError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_state_semantics() {
        assert!(ConnectionState::Connected.is_connected());
        assert!(!ConnectionState::WaitingClient.is_connected());
        assert!(!ConnectionState::VersionMismatch.is_connected());
    }
}
