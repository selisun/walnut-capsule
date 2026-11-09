//! 快照接收门：序列号、时间戳、过期判定。

use screen_core::Snapshot;

use crate::error::DataError;
use crate::state::{effective_threshold_ms, DataState, DataStatus};

const FUTURE_TOLERANCE_MS: i64 = 5_000;

#[derive(Debug, Clone)]
pub struct SnapshotGate {
    threshold_ms: i64,
    last_seq: Option<u64>,
    last_ts_ms: Option<i64>,
    state: DataState,
    dropped: u64,
}

impl SnapshotGate {
    pub fn new(config_stale_ms: i64, period_ms: i64) -> Result<Self, DataError> {
        if config_stale_ms < 0 || period_ms <= 0 {
            return Err(DataError::InvalidThreshold("stale/period"));
        }
        Ok(Self {
            threshold_ms: effective_threshold_ms(config_stale_ms, period_ms),
            last_seq: None,
            last_ts_ms: None,
            state: DataState::WaitingClient,
            dropped: 0,
        })
    }

    pub fn threshold_ms(&self) -> i64 {
        self.threshold_ms
    }

    pub fn state(&self) -> DataState {
        self.state
    }

    pub fn status(&self) -> DataStatus {
        DataStatus {
            state: self.state,
            last_seq: self.last_seq,
            last_ts_ms: self.last_ts_ms,
            threshold_ms: self.threshold_ms,
            dropped: self.dropped,
        }
    }

    /// 接收一帧快照。序列号必须严格递增；时间戳不得明显来自未来。
    pub fn accept(&mut self, snapshot: &Snapshot, now_ms: i64) -> Result<(), DataError> {
        if let Some(last) = self.last_seq {
            if snapshot.seq == last {
                self.dropped += 1;
                return Err(DataError::DuplicateSeq { seq: last });
            }
            if snapshot.seq < last {
                self.dropped += 1;
                return Err(DataError::OutOfOrder {
                    last,
                    got: snapshot.seq,
                });
            }
        }
        if snapshot.ts_ms > now_ms.saturating_add(FUTURE_TOLERANCE_MS) {
            return Err(DataError::TimestampInFuture {
                ts_ms: snapshot.ts_ms,
                now_ms,
            });
        }

        self.last_seq = Some(snapshot.seq);
        self.last_ts_ms = Some(snapshot.ts_ms);
        self.state = if self.is_stale_at(snapshot.ts_ms, now_ms) {
            DataState::Stale
        } else {
            DataState::Ready
        };
        Ok(())
    }

    /// 运行期刷新周期变化：只重算过期阈值，保留序列号/最后时间戳与当前状态。
    /// 不清空快照，避免热更新瞬间额外渲染一帧「等待客户端」占位画面。
    pub fn update_threshold(
        &mut self,
        config_stale_ms: i64,
        period_ms: i64,
    ) -> Result<(), DataError> {
        if config_stale_ms < 0 || period_ms <= 0 {
            return Err(DataError::InvalidThreshold("stale/period"));
        }
        self.threshold_ms = effective_threshold_ms(config_stale_ms, period_ms);
        Ok(())
    }

    /// 定时检查（无新数据时由调度器调用）。
    pub fn tick(&mut self, now_ms: i64) -> DataState {
        if self.state == DataState::VersionMismatch {
            return self.state;
        }
        self.state = match self.last_ts_ms {
            None => DataState::WaitingClient,
            Some(ts) if self.is_stale_at(ts, now_ms) => DataState::Stale,
            Some(_) => DataState::Ready,
        };
        self.state
    }

    /// 管道断开/客户端退出：进入等待，不把旧值当实时。
    /// 必须清掉 `last_ts_ms`——否则下一次 `tick` 会按旧时间戳重算出 `Ready`，
    /// 而快照已由调用方清除，导致「Ready 但无数据」的渲染失败（T5 实测）。
    pub fn mark_client_disconnected(&mut self) {
        self.last_ts_ms = None;
        if self.state != DataState::VersionMismatch {
            self.state = DataState::WaitingClient;
        }
    }

    /// 客户端重启（版本协商重新开始）：丢弃旧序列基线。
    pub fn reset_for_new_client(&mut self) {
        self.last_seq = None;
        self.last_ts_ms = None;
        self.state = DataState::WaitingClient;
    }

    /// IPC 主版本不兼容：拒绝后续数据。
    pub fn mark_version_mismatch(&mut self) {
        self.state = DataState::VersionMismatch;
        self.last_seq = None;
        self.last_ts_ms = None;
    }

    fn is_stale_at(&self, ts_ms: i64, now_ms: i64) -> bool {
        now_ms.saturating_sub(ts_ms) > self.threshold_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(seq: u64, ts_ms: i64) -> Snapshot {
        Snapshot::new(seq, ts_ms)
    }

    #[test]
    fn first_snapshot_is_ready() {
        let mut gate = SnapshotGate::new(3_000, 1_000).expect("gate");
        gate.accept(&snap(1, 10_000), 10_100).expect("accept");
        assert_eq!(gate.state(), DataState::Ready);
    }

    #[test]
    fn stale_after_threshold() {
        let mut gate = SnapshotGate::new(3_000, 1_000).expect("gate");
        gate.accept(&snap(1, 10_000), 10_000).expect("accept");
        assert_eq!(gate.tick(13_001), DataState::Stale);
        assert_eq!(gate.tick(12_999), DataState::Ready);
    }

    #[test]
    fn update_threshold_keeps_sequence_and_state() {
        let mut gate = SnapshotGate::new(3_000, 1_000).expect("gate");
        gate.accept(&snap(7, 10_000), 10_000).expect("accept");
        assert_eq!(gate.state(), DataState::Ready);

        gate.update_threshold(3_000, 5_000).expect("update");
        assert_eq!(gate.threshold_ms(), 15_000);
        assert_eq!(gate.status().last_seq, Some(7));
        assert_eq!(gate.tick(20_000), DataState::Ready);
        assert_eq!(gate.tick(25_001), DataState::Stale);

        assert!(gate.update_threshold(3_000, 0).is_err());
        assert!(gate.update_threshold(-1, 1_000).is_err());
    }

    #[test]
    fn waiting_without_client() {
        let mut gate = SnapshotGate::new(3_000, 1_000).expect("gate");
        assert_eq!(gate.tick(1_000), DataState::WaitingClient);
    }

    #[test]
    fn duplicate_and_out_of_order_are_rejected() {
        let mut gate = SnapshotGate::new(3_000, 1_000).expect("gate");
        gate.accept(&snap(5, 1_000), 1_000).expect("accept");
        assert_eq!(
            gate.accept(&snap(5, 1_100), 1_100),
            Err(DataError::DuplicateSeq { seq: 5 })
        );
        assert_eq!(
            gate.accept(&snap(4, 1_200), 1_200),
            Err(DataError::OutOfOrder { last: 5, got: 4 })
        );
        assert_eq!(gate.status().dropped, 2);
    }

    #[test]
    fn future_timestamp_is_rejected() {
        let mut gate = SnapshotGate::new(3_000, 1_000).expect("gate");
        let err = gate.accept(&snap(1, 20_000), 10_000).unwrap_err();
        assert!(matches!(err, DataError::TimestampInFuture { .. }));
        assert_eq!(gate.state(), DataState::WaitingClient);
    }

    #[test]
    fn disconnect_marks_waiting_even_if_recent() {
        let mut gate = SnapshotGate::new(3_000, 1_000).expect("gate");
        gate.accept(&snap(1, 10_000), 10_000).expect("accept");
        gate.mark_client_disconnected();
        assert_eq!(gate.state(), DataState::WaitingClient);
    }

    #[test]
    fn disconnect_stays_waiting_on_following_tick() {
        // 回归：只改 state 不清时间戳时，下一拍 tick 会按旧 ts 重算成 Ready，
        // 而快照已被清除 → 出现「Ready 但无数据」的渲染失败（T5 实测 2 次）。
        let mut gate = SnapshotGate::new(3_000, 1_000).expect("gate");
        gate.accept(&snap(1, 10_000), 10_000).expect("accept");
        assert_eq!(gate.state(), DataState::Ready);
        gate.mark_client_disconnected();
        assert_eq!(gate.tick(10_100), DataState::WaitingClient);
        assert_eq!(gate.tick(11_000), DataState::WaitingClient);
        assert_eq!(gate.tick(20_000), DataState::WaitingClient);
    }

    #[test]
    fn reset_allows_new_sequence_baseline() {
        let mut gate = SnapshotGate::new(3_000, 1_000).expect("gate");
        gate.accept(&snap(9, 10_000), 10_000).expect("accept");
        gate.reset_for_new_client();
        gate.accept(&snap(1, 11_000), 11_000).expect("accept");
        assert_eq!(gate.state(), DataState::Ready);
    }

    #[test]
    fn version_mismatch_blocks_recovery() {
        let mut gate = SnapshotGate::new(3_000, 1_000).expect("gate");
        gate.accept(&snap(1, 10_000), 10_000).expect("accept");
        gate.mark_version_mismatch();
        assert_eq!(gate.tick(10_000), DataState::VersionMismatch);
        gate.mark_client_disconnected();
        assert_eq!(gate.state(), DataState::VersionMismatch);
    }

    #[test]
    fn invalid_threshold_is_rejected() {
        assert!(SnapshotGate::new(0, 0).is_err());
        assert!(SnapshotGate::new(-1, 1_000).is_err());
    }
}
