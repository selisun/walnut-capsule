//! 设备生命周期状态。状态转移规则与 `03` 第 8 节一致。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Discovered,
    Identified,
    Opening,
    Initializing,
    Ready,
    Streaming,
    Degraded,
    Disconnected,
    Busy,
    Error,
    Unsupported,
}

impl DeviceState {
    pub fn is_attached(self) -> bool {
        matches!(
            self,
            DeviceState::Ready | DeviceState::Streaming | DeviceState::Degraded
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, DeviceState::Unsupported)
    }

    /// 是否允许渲染/发送帧。
    pub fn can_stream(self) -> bool {
        matches!(
            self,
            DeviceState::Ready | DeviceState::Streaming | DeviceState::Degraded
        )
    }
}

/// 合法状态转移（`01` 状态图 + `03` 恢复规则）。
pub fn can_transition(from: DeviceState, to: DeviceState) -> bool {
    use DeviceState::*;
    match (from, to) {
        (Discovered, Identified) | (Discovered, Unsupported) => true,
        (Identified, Opening) | (Identified, Unsupported) => true,
        (Opening, Initializing) | (Opening, Error) | (Opening, Busy) => true,
        (Initializing, Ready) | (Initializing, Error) => true,
        (Ready, Streaming) | (Ready, Disconnected) | (Ready, Error) => true,
        (Streaming, Degraded) | (Streaming, Disconnected) | (Streaming, Error) => true,
        (Degraded, Streaming) | (Degraded, Disconnected) | (Degraded, Error) => true,
        (Disconnected, Discovered) => true,
        (Error, Opening) | (Busy, Opening) => true,
        // 显式允许复位到 Disconnected（拔线/关闭）
        (_, Disconnected) => true,
        // 同一状态重复设置是幂等的
        (a, b) if a == b => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_transition_is_rejected() {
        assert!(!can_transition(DeviceState::Ready, DeviceState::Discovered));
        assert!(!can_transition(
            DeviceState::Unsupported,
            DeviceState::Ready
        ));
    }

    #[test]
    fn documented_happy_path_is_allowed() {
        let path = [
            (DeviceState::Discovered, DeviceState::Identified),
            (DeviceState::Identified, DeviceState::Opening),
            (DeviceState::Opening, DeviceState::Initializing),
            (DeviceState::Initializing, DeviceState::Ready),
            (DeviceState::Ready, DeviceState::Streaming),
            (DeviceState::Streaming, DeviceState::Degraded),
            (DeviceState::Degraded, DeviceState::Streaming),
            (DeviceState::Streaming, DeviceState::Disconnected),
            (DeviceState::Disconnected, DeviceState::Discovered),
        ];
        for (from, to) in path {
            assert!(can_transition(from, to), "{from:?} -> {to:?}");
        }
    }

    #[test]
    fn only_attached_states_can_stream() {
        assert!(DeviceState::Ready.can_stream());
        assert!(DeviceState::Streaming.can_stream());
        assert!(DeviceState::Degraded.can_stream());
        assert!(!DeviceState::Busy.can_stream());
        assert!(!DeviceState::Disconnected.can_stream());
    }
}
