//! 消息组装与 payload 编解码。只允许固定消息类型，不做通用 RPC。

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::IpcError;
use crate::header::{Header, MessageKind, HEADER_LEN, MAX_FRAME_BYTES};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub header: Header,
    pub payload: Vec<u8>,
}

impl Message {
    pub fn new(kind: MessageKind, seq: u64, payload: Vec<u8>) -> Result<Self, IpcError> {
        let header = Header::new(kind, seq, payload.len())?;
        Ok(Self { header, payload })
    }

    pub fn with_payload<T: Serialize>(
        kind: MessageKind,
        seq: u64,
        value: &T,
    ) -> Result<Self, IpcError> {
        let payload = serde_json::to_vec(value).map_err(|e| IpcError::Json(e.to_string()))?;
        Self::new(kind, seq, payload)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, IpcError> {
        if self.header.payload_len as usize != self.payload.len() {
            return Err(IpcError::LengthMismatch {
                header: self.header.payload_len as usize,
                actual: self.payload.len(),
            });
        }
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len());
        out.extend_from_slice(&self.header.encode());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, IpcError> {
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(IpcError::PayloadTooLarge {
                max: MAX_FRAME_BYTES,
                got: bytes.len(),
            });
        }
        let header = Header::decode(bytes)?;
        let expected = HEADER_LEN + header.payload_len as usize;
        if bytes.len() != expected {
            return Err(IpcError::LengthMismatch {
                header: header.payload_len as usize,
                actual: bytes.len().saturating_sub(HEADER_LEN),
            });
        }
        Ok(Self {
            header,
            payload: bytes[HEADER_LEN..].to_vec(),
        })
    }

    pub fn decode_payload<T: DeserializeOwned>(&self) -> Result<T, IpcError> {
        serde_json::from_slice(&self.payload).map_err(|e| IpcError::Json(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Demo {
        value: u32,
    }

    #[test]
    fn message_roundtrip_with_payload() {
        let msg =
            Message::with_payload(MessageKind::Snapshot, 7, &Demo { value: 123 }).expect("encode");
        let bytes = msg.to_bytes().expect("bytes");
        let decoded = Message::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded.header.seq, 7);
        let demo: Demo = decoded.decode_payload().expect("payload");
        assert_eq!(demo.value, 123);
    }

    #[test]
    fn length_field_mismatch_is_rejected() {
        let mut msg = Message::new(MessageKind::Snapshot, 1, vec![1, 2, 3]).expect("msg");
        msg.payload.push(4);
        assert!(matches!(
            msg.to_bytes(),
            Err(IpcError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn truncated_message_is_rejected() {
        let msg = Message::new(MessageKind::Snapshot, 1, vec![1, 2, 3]).expect("msg");
        let bytes = msg.to_bytes().expect("bytes");
        assert!(Message::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn extra_bytes_after_payload_is_rejected() {
        let msg = Message::new(MessageKind::Snapshot, 1, vec![1, 2, 3]).expect("msg");
        let mut bytes = msg.to_bytes().expect("bytes");
        bytes.push(0xFF);
        assert!(Message::from_bytes(&bytes).is_err());
    }
}
