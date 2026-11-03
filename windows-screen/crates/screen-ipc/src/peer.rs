//! 对端身份校验：映像文件名白名单。DACL 负责用户隔离，本校验负责进程身份。

use crate::error::IpcError;
use crate::transport::{PeerInfo, PeerVerifier};

/// 只接受映像文件名等于 `expected` 的对端（大小写不敏感）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageNameVerifier {
    expected: String,
}

impl ImageNameVerifier {
    pub fn new(expected: impl Into<String>) -> Self {
        Self {
            expected: expected.into(),
        }
    }

    pub fn expected(&self) -> &str {
        &self.expected
    }
}

impl PeerVerifier for ImageNameVerifier {
    fn verify(&self, peer: &PeerInfo) -> Result<(), IpcError> {
        let path = peer
            .image_path
            .as_deref()
            .ok_or(IpcError::Refused("无法获取对端映像路径"))?;
        let name = path
            .rsplit(['\\', '/'])
            .find(|part| !part.is_empty())
            .ok_or(IpcError::Refused("对端映像路径为空"))?;
        if name.eq_ignore_ascii_case(&self.expected) {
            Ok(())
        } else {
            Err(IpcError::Refused("对端映像名不在白名单"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(path: &str) -> PeerInfo {
        PeerInfo {
            process_id: 42,
            user_sid: None,
            image_path: Some(path.to_string()),
            image_sha256: None,
        }
    }

    #[test]
    fn accepts_matching_image_name() {
        let verifier = ImageNameVerifier::new("hetao-capsule.exe");
        assert!(verifier
            .verify(&peer(r"C:\Program Files\核桃胶囊\hetao-capsule.exe"))
            .is_ok());
        assert!(verifier.verify(&peer("hetao-capsule.exe")).is_ok());
    }

    #[test]
    fn comparison_is_case_insensitive() {
        let verifier = ImageNameVerifier::new("HETAO-CAPSULE.EXE");
        assert!(verifier.verify(&peer(r"C:\apps\hetao-capsule.exe")).is_ok());
    }

    #[test]
    fn rejects_other_image() {
        let verifier = ImageNameVerifier::new("hetao-capsule.exe");
        let err = verifier.verify(&peer(r"C:\Windows\evil.exe")).unwrap_err();
        assert!(matches!(err, IpcError::Refused(_)));
    }

    #[test]
    fn rejects_missing_image_path() {
        let verifier = ImageNameVerifier::new("hetao-capsule.exe");
        let peer = PeerInfo {
            process_id: 7,
            user_sid: None,
            image_path: None,
            image_sha256: None,
        };
        assert!(verifier.verify(&peer).is_err());
    }
}
