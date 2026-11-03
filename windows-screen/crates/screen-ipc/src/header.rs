//! 消息头与版本协商。字节布局见 `01` §10.4：
//!
//! ```text
//! magic(4) | major(2) | minor(2) | kind(1) | flags(1) | seq(8) | payload_len(4)
//! ```
//!
//! 总计 22 字节，小端序；单帧 payload 上限 256KB - 22。

use crate::error::IpcError;

pub const MAGIC: [u8; 4] = *b"HTSP";
pub const HEADER_LEN: usize = 22;
pub const SCHEMA_MAJOR: u16 = 1;
pub const SCHEMA_MINOR: u16 = 0;
pub const MAX_FRAME_BYTES: usize = 256 * 1024;
pub const MAX_PAYLOAD_BYTES: usize = MAX_FRAME_BYTES - HEADER_LEN;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Hello = 1,
    Ack = 2,
    Snapshot = 3,
    ConfigChanged = 4,
    StatusQuery = 5,
    Bye = 6,
}

impl MessageKind {
    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for MessageKind {
    type Error = IpcError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Hello),
            2 => Ok(Self::Ack),
            3 => Ok(Self::Snapshot),
            4 => Ok(Self::ConfigChanged),
            5 => Ok(Self::StatusQuery),
            6 => Ok(Self::Bye),
            other => Err(IpcError::UnknownKind(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
}

impl Version {
    pub const CURRENT: Self = Self {
        major: SCHEMA_MAJOR,
        minor: SCHEMA_MINOR,
    };

    pub fn is_compatible_with(self, other: Version) -> bool {
        self.major == other.major
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: Version,
    pub kind: MessageKind,
    pub flags: u8,
    pub seq: u64,
    pub payload_len: u32,
}

impl Header {
    pub fn new(kind: MessageKind, seq: u64, payload_len: usize) -> Result<Self, IpcError> {
        if payload_len > MAX_PAYLOAD_BYTES {
            return Err(IpcError::PayloadTooLarge {
                max: MAX_PAYLOAD_BYTES,
                got: payload_len,
            });
        }
        Ok(Self {
            version: Version::CURRENT,
            kind,
            flags: 0,
            seq,
            payload_len: payload_len as u32,
        })
    }

    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0..4].copy_from_slice(&MAGIC);
        out[4..6].copy_from_slice(&self.version.major.to_le_bytes());
        out[6..8].copy_from_slice(&self.version.minor.to_le_bytes());
        out[8] = self.kind.to_u8();
        out[9] = self.flags;
        out[10..18].copy_from_slice(&self.seq.to_le_bytes());
        out[18..22].copy_from_slice(&self.payload_len.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, IpcError> {
        if bytes.len() < HEADER_LEN {
            return Err(IpcError::TooShort {
                need: HEADER_LEN,
                got: bytes.len(),
            });
        }
        if bytes[0..4] != MAGIC {
            return Err(IpcError::BadMagic);
        }
        let major = u16::from_le_bytes([bytes[4], bytes[5]]);
        let minor = u16::from_le_bytes([bytes[6], bytes[7]]);
        let version = Version { major, minor };
        if !Version::CURRENT.is_compatible_with(version) {
            return Err(IpcError::UnsupportedMajor {
                expected: SCHEMA_MAJOR,
                got: major,
            });
        }
        let kind = MessageKind::try_from(bytes[8])?;
        let flags = bytes[9];
        let seq = u64::from_le_bytes([
            bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17],
        ]);
        let payload_len = u32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]) as usize;
        if payload_len > MAX_PAYLOAD_BYTES {
            return Err(IpcError::PayloadTooLarge {
                max: MAX_PAYLOAD_BYTES,
                got: payload_len,
            });
        }
        Ok(Self {
            version,
            kind,
            flags,
            seq,
            payload_len: payload_len as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let h = Header::new(MessageKind::Snapshot, 42, 10).expect("header");
        let bytes = h.encode();
        assert_eq!(bytes.len(), HEADER_LEN);
        let decoded = Header::decode(&bytes).expect("decode");
        assert_eq!(decoded, h);
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut bytes = Header::new(MessageKind::Snapshot, 1, 0)
            .expect("header")
            .encode();
        bytes[0] = b'X';
        assert_eq!(Header::decode(&bytes), Err(IpcError::BadMagic));
    }

    #[test]
    fn major_mismatch_is_rejected() {
        let mut bytes = Header::new(MessageKind::Snapshot, 1, 0)
            .expect("header")
            .encode();
        bytes[4] = 2;
        assert!(matches!(
            Header::decode(&bytes),
            Err(IpcError::UnsupportedMajor { .. })
        ));
    }

    #[test]
    fn unknown_kind_is_rejected() {
        let mut bytes = Header::new(MessageKind::Snapshot, 1, 0)
            .expect("header")
            .encode();
        bytes[8] = 99;
        assert_eq!(Header::decode(&bytes), Err(IpcError::UnknownKind(99)));
    }

    #[test]
    fn oversized_payload_is_rejected_before_encoding() {
        assert!(Header::new(MessageKind::Snapshot, 1, MAX_PAYLOAD_BYTES + 1).is_err());
    }

    #[test]
    fn version_compatibility_only_checks_major() {
        let v = Version { major: 1, minor: 9 };
        assert!(Version::CURRENT.is_compatible_with(v));
        let v = Version { major: 2, minor: 0 };
        assert!(!Version::CURRENT.is_compatible_with(v));
    }
}
