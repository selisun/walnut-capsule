//! 数据快照模型。只描述主客户端推送的数据，不含 NAS 地址/Token。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CpuMetrics {
    pub usage_pct: Option<f32>,
    pub temp_c: Option<f32>,
    pub load_1m: Option<f32>,
    pub load_5m: Option<f32>,
    pub load_15m: Option<f32>,
    pub load_30m: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryMetrics {
    pub usage_pct: Option<f32>,
    pub used_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub cached_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuMetrics {
    pub name: Option<String>,
    pub usage_pct: Option<f32>,
    pub temp_c: Option<f32>,
    pub vram_used_bytes: Option<u64>,
    pub vram_total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NetworkMetrics {
    pub iface: Option<String>,
    pub up_bps: Option<u64>,
    pub down_bps: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiskMetrics {
    pub device: Option<String>,
    pub read_bps: Option<u64>,
    pub write_bps: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StorageVolume {
    pub name: String,
    pub used_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NasInfo {
    pub name: Option<String>,
    pub online: Option<bool>,
    /// 显示用的地址（`host` 或 `host:port`），仅用于顶栏，不含凭据。
    #[serde(default)]
    pub addr: Option<String>,
}

/// 电脑本机（运行胶囊的这台机器）指标，供双显主题右列使用。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PcMetrics {
    pub cpu_pct: Option<f32>,
    pub mem_pct: Option<f32>,
}

/// 指标集合：`None` 表示该项缺失，渲染层必须整项隐藏（见 `06`/`09`）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub cpu: Option<CpuMetrics>,
    pub memory: Option<MemoryMetrics>,
    pub gpu: Option<GpuMetrics>,
    pub network: Option<NetworkMetrics>,
    pub disk: Option<DiskMetrics>,
    #[serde(default)]
    pub storage: Vec<StorageVolume>,
}

impl Metrics {
    pub fn is_empty(&self) -> bool {
        self.cpu.is_none()
            && self.memory.is_none()
            && self.gpu.is_none()
            && self.network.is_none()
            && self.disk.is_none()
            && self.storage.is_empty()
    }
}

/// 主客户端推送的一帧快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// 单调递增序列号；由 `screen-data` 校验。
    pub seq: u64,
    /// 客户端采样时间（Unix 毫秒）。用于过期判定。
    pub ts_ms: i64,
    #[serde(default)]
    pub nas: NasInfo,
    #[serde(default)]
    pub metrics: Metrics,
    /// 电脑本机指标；旧版客户端不推送时保持全空。
    #[serde(default)]
    pub pc: PcMetrics,
}

impl Snapshot {
    pub fn new(seq: u64, ts_ms: i64) -> Self {
        Self {
            seq,
            ts_ms,
            nas: NasInfo::default(),
            metrics: Metrics::default(),
            pc: PcMetrics::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.metrics.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_metrics_are_detected() {
        let snap = Snapshot::new(1, 1000);
        assert!(snap.is_empty());
    }

    #[test]
    fn missing_fields_stay_none() {
        let json = r#"{"seq":7,"ts_ms":1000,"metrics":{"cpu":{"usage_pct":null}}}"#;
        let snap: Snapshot = serde_json::from_str(json).expect("snapshot json");
        assert_eq!(snap.seq, 7);
        let cpu = snap.metrics.cpu.expect("cpu present");
        assert_eq!(cpu.usage_pct, None);
        assert!(snap.metrics.memory.is_none());
    }
}
