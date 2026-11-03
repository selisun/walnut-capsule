//! 数据层错误。

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    DuplicateSeq { seq: u64 },
    OutOfOrder { last: u64, got: u64 },
    InvalidThreshold(&'static str),
    TimestampInFuture { ts_ms: i64, now_ms: i64 },
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataError::DuplicateSeq { seq } => write!(f, "重复的序列号：{seq}"),
            DataError::OutOfOrder { last, got } => {
                write!(f, "序列号倒退：上一帧 {last}，本次 {got}")
            }
            DataError::InvalidThreshold(name) => write!(f, "非法阈值：{name}"),
            DataError::TimestampInFuture { ts_ms, now_ms } => {
                write!(f, "时间戳来自未来：{ts_ms} > {now_ms}")
            }
        }
    }
}

impl std::error::Error for DataError {}
