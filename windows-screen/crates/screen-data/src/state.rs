//! 数据状态与新鲜度阈值。

use serde::{Deserialize, Serialize};

/// 小屏的数据状态。只有 `Ready` 可以按实时数据渲染。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataState {
    WaitingClient,
    Ready,
    Stale,
    VersionMismatch,
}

impl DataState {
    pub fn is_live(self) -> bool {
        matches!(self, DataState::Ready)
    }
}

/// 过期阈值：`max(3s, 3×推送周期, 配置值)`。
pub fn effective_threshold_ms(config_stale_ms: i64, period_ms: i64) -> i64 {
    config_stale_ms.max(period_ms.saturating_mul(3)).max(3_000)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataStatus {
    pub state: DataState,
    pub last_seq: Option<u64>,
    pub last_ts_ms: Option<i64>,
    pub threshold_ms: i64,
    pub dropped: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_is_never_below_three_seconds() {
        assert_eq!(effective_threshold_ms(1_000, 1_000), 3_000);
        assert_eq!(effective_threshold_ms(10_000, 1_000), 10_000);
        assert_eq!(effective_threshold_ms(3_000, 5_000), 15_000);
    }
}
