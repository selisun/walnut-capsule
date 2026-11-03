//! 指标绑定与显示格式化。规则：缺失指标整卡隐藏；缺失数值显示 `--`，不得用 0 填充。

use screen_core::{ComponentKind, Metrics};

/// 渲染层可直接使用的指标视图。所有数值都是 `Option`，`None` 表示缺失。
#[derive(Debug, Clone, PartialEq)]
pub enum MetricView {
    Cpu {
        usage: Option<f32>,
        temp: Option<f32>,
    },
    Memory {
        usage: Option<f32>,
        used: Option<u64>,
        total: Option<u64>,
    },
    Gpu {
        name: Option<String>,
        usage: Option<f32>,
        temp: Option<f32>,
        vram: Option<(u64, u64)>,
    },
    Network {
        iface: Option<String>,
        up: Option<u64>,
        down: Option<u64>,
    },
    Disk {
        device: Option<String>,
        read: Option<u64>,
        write: Option<u64>,
    },
}

impl MetricView {
    pub fn kind(&self) -> ComponentKind {
        match self {
            MetricView::Cpu { .. } => ComponentKind::Cpu,
            MetricView::Memory { .. } => ComponentKind::Memory,
            MetricView::Gpu { .. } => ComponentKind::Gpu,
            MetricView::Network { .. } => ComponentKind::Network,
            MetricView::Disk { .. } => ComponentKind::Disk,
        }
    }

    /// 卡片主数值（大字）。
    pub fn primary(&self) -> String {
        match self {
            MetricView::Cpu { usage, .. } | MetricView::Memory { usage, .. } => format_pct(*usage),
            MetricView::Gpu { usage, .. } => format_pct(*usage),
            MetricView::Network { up, down, .. } => {
                if up.is_none() && down.is_none() {
                    "--".to_string()
                } else {
                    format!("下 {}", format_rate_opt(*down))
                }
            }
            MetricView::Disk { read, write, .. } => {
                if read.is_none() && write.is_none() {
                    "--".to_string()
                } else {
                    format!("读 {}", format_rate_opt(*read))
                }
            }
        }
    }

    /// 卡片副信息（小字）；空串表示不绘制。
    pub fn secondary(&self) -> String {
        match self {
            MetricView::Cpu { temp, .. } => format_temp(*temp),
            MetricView::Memory { used, total, .. } => format_pair(*used, *total),
            MetricView::Gpu { temp, vram, .. } => {
                let mut parts = Vec::new();
                if let Some((used, total)) = vram {
                    parts.push(format_pair(Some(*used), Some(*total)));
                }
                if temp.is_some() {
                    parts.push(format_temp(*temp));
                }
                parts.join(" ")
            }
            MetricView::Network { up, down, .. } => {
                if up.is_none() && down.is_none() {
                    String::new()
                } else {
                    format!("上 {}", format_rate_opt(*up))
                }
            }
            MetricView::Disk { read, write, .. } => {
                if read.is_none() && write.is_none() {
                    String::new()
                } else {
                    format!("写 {}", format_rate_opt(*write))
                }
            }
        }
    }
}

/// 默认中文标签；主题 `components.styles.<kind>.label` 可覆盖。
pub fn default_label(kind: ComponentKind) -> &'static str {
    match kind {
        ComponentKind::Cpu => "CPU",
        ComponentKind::Memory => "内存",
        ComponentKind::Gpu => "显卡",
        ComponentKind::Network => "网络",
        ComponentKind::Disk => "磁盘",
        ComponentKind::Storage => "存储",
        ComponentKind::Temp => "温度",
        ComponentKind::Clock => "时钟",
    }
}

/// 按 `visible` 顺序绑定；对应 `Metrics` 项为 `None` 时不产出视图。
pub fn bind(metrics: &Metrics, visible: &[ComponentKind]) -> Vec<(ComponentKind, MetricView)> {
    let mut out = Vec::new();
    for kind in visible {
        let view = match kind {
            ComponentKind::Cpu => metrics.cpu.as_ref().map(|cpu| MetricView::Cpu {
                usage: cpu.usage_pct,
                temp: cpu.temp_c,
            }),
            ComponentKind::Memory => metrics.memory.as_ref().map(|mem| MetricView::Memory {
                usage: mem.usage_pct,
                used: mem.used_bytes,
                total: mem.total_bytes,
            }),
            ComponentKind::Gpu => metrics.gpu.as_ref().map(|gpu| MetricView::Gpu {
                name: gpu.name.clone(),
                usage: gpu.usage_pct,
                temp: gpu.temp_c,
                vram: match (gpu.vram_used_bytes, gpu.vram_total_bytes) {
                    (Some(used), Some(total)) => Some((used, total)),
                    _ => None,
                },
            }),
            ComponentKind::Network => metrics.network.as_ref().map(|net| MetricView::Network {
                iface: net.iface.clone(),
                up: net.up_bps,
                down: net.down_bps,
            }),
            ComponentKind::Disk => metrics.disk.as_ref().map(|disk| MetricView::Disk {
                device: disk.device.clone(),
                read: disk.read_bps,
                write: disk.write_bps,
            }),
            ComponentKind::Storage | ComponentKind::Temp | ComponentKind::Clock => None,
        };
        if let Some(view) = view {
            out.push((*kind, view));
        }
    }
    out
}

/// 百分比；`None` 或非有限值一律 `--`，绝不显示 0%。
pub fn format_pct(value: Option<f32>) -> String {
    match value {
        Some(v) if v.is_finite() => format!("{:.0}%", v.clamp(0.0, 100.0)),
        _ => "--".to_string(),
    }
}

/// 字节数：B / KB / MB / GB / TB。
pub fn format_bytes(bytes: u64) -> String {
    format_scaled(bytes, "B")
}

/// 速率：B/s / KB/s / MB/s / GB/s。
pub fn format_rate(bps: u64) -> String {
    format_scaled(bps, "B/s")
}

fn format_scaled(value: u64, base_unit: &str) -> String {
    let units: [&str; 5] = [base_unit, "KB", "MB", "GB", "TB"];
    let mut scaled = value as f64;
    let mut unit = 0;
    while scaled >= 1024.0 && unit + 1 < units.len() {
        scaled /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{value} {base_unit}");
    }
    let suffix = if base_unit == "B/s" {
        format!("{}/s", units[unit])
    } else {
        units[unit].to_string()
    };
    format!("{scaled:.1} {suffix}")
}

fn format_rate_opt(value: Option<u64>) -> String {
    match value {
        Some(v) => format_rate(v),
        None => "--".to_string(),
    }
}

fn format_pair(used: Option<u64>, total: Option<u64>) -> String {
    match (used, total) {
        (Some(u), Some(t)) => format!("{} / {}", format_bytes(u), format_bytes(t)),
        (Some(u), None) => format_bytes(u),
        (None, Some(t)) => format!("-- / {}", format_bytes(t)),
        (None, None) => "--".to_string(),
    }
}

/// 温度；半角 `C`，避免字体缺字。
pub fn format_temp(value: Option<f32>) -> String {
    match value {
        Some(v) if v.is_finite() => format!("{:.0}°C", v),
        _ => "--".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use screen_core::{CpuMetrics, DiskMetrics, GpuMetrics, MemoryMetrics, NetworkMetrics};

    fn all_visible() -> Vec<ComponentKind> {
        vec![
            ComponentKind::Cpu,
            ComponentKind::Memory,
            ComponentKind::Gpu,
            ComponentKind::Network,
            ComponentKind::Disk,
        ]
    }

    #[test]
    fn empty_metrics_bind_to_nothing() {
        assert!(bind(&Metrics::default(), &all_visible()).is_empty());
    }

    #[test]
    fn missing_usage_is_dash_not_zero() {
        let metrics = Metrics {
            cpu: Some(CpuMetrics {
                usage_pct: None,
                temp_c: Some(45.0),
                ..CpuMetrics::default()
            }),
            ..Metrics::default()
        };
        let bound = bind(&metrics, &[ComponentKind::Cpu]);
        assert_eq!(bound.len(), 1);
        let view = &bound[0].1;
        assert_eq!(view.primary(), "--");
        assert_eq!(view.secondary(), "45°C");
        assert_ne!(view.primary(), "0%");
    }

    #[test]
    fn only_existing_metrics_produce_views() {
        let metrics = Metrics {
            cpu: Some(CpuMetrics {
                usage_pct: Some(42.0),
                ..CpuMetrics::default()
            }),
            ..Metrics::default()
        };
        let bound = bind(&metrics, &all_visible());
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].0, ComponentKind::Cpu);
    }

    #[test]
    fn bind_respects_visible_order() {
        let metrics = Metrics {
            cpu: Some(CpuMetrics {
                usage_pct: Some(10.0),
                ..CpuMetrics::default()
            }),
            memory: Some(MemoryMetrics {
                usage_pct: Some(50.0),
                used_bytes: Some(4 * 1024 * 1024 * 1024),
                total_bytes: Some(8 * 1024 * 1024 * 1024),
                ..MemoryMetrics::default()
            }),
            ..Metrics::default()
        };
        let bound = bind(&metrics, &[ComponentKind::Memory, ComponentKind::Cpu]);
        assert_eq!(
            bound.iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
            vec![ComponentKind::Memory, ComponentKind::Cpu]
        );
        assert_eq!(bound[0].1.primary(), "50%");
        assert_eq!(bound[0].1.secondary(), "4.0 GB / 8.0 GB");
    }

    #[test]
    fn memory_pair_handles_missing_halves() {
        let metrics = Metrics {
            memory: Some(MemoryMetrics {
                usage_pct: None,
                used_bytes: None,
                total_bytes: Some(8 * 1024 * 1024 * 1024),
                ..MemoryMetrics::default()
            }),
            ..Metrics::default()
        };
        let bound = bind(&metrics, &[ComponentKind::Memory]);
        assert_eq!(bound[0].1.primary(), "--");
        assert_eq!(bound[0].1.secondary(), "-- / 8.0 GB");
    }

    #[test]
    fn gpu_network_disk_views() {
        let metrics = Metrics {
            gpu: Some(GpuMetrics {
                name: Some("GPU".into()),
                usage_pct: Some(30.0),
                temp_c: Some(60.0),
                vram_used_bytes: Some(2 * 1024 * 1024 * 1024),
                vram_total_bytes: Some(8 * 1024 * 1024 * 1024),
            }),
            network: Some(NetworkMetrics {
                iface: Some("eth0".into()),
                up_bps: Some(1024),
                down_bps: Some(1024 * 1024),
            }),
            disk: Some(DiskMetrics {
                device: Some("disk0".into()),
                read_bps: Some(2 * 1024 * 1024),
                write_bps: Some(512 * 1024),
            }),
            ..Metrics::default()
        };
        let bound = bind(
            &metrics,
            &[
                ComponentKind::Gpu,
                ComponentKind::Network,
                ComponentKind::Disk,
            ],
        );
        assert_eq!(bound[0].1.primary(), "30%");
        assert!(bound[0].1.secondary().contains("2.0 GB / 8.0 GB"));
        assert!(bound[0].1.secondary().contains("60°C"));
        assert_eq!(bound[1].1.primary(), "下 1.0 MB/s");
        assert_eq!(bound[1].1.secondary(), "上 1.0 KB/s");
        assert_eq!(bound[2].1.primary(), "读 2.0 MB/s");
        assert_eq!(bound[2].1.secondary(), "写 512.0 KB/s");
    }

    #[test]
    fn missing_network_and_disk_show_dash() {
        let metrics = Metrics {
            network: Some(NetworkMetrics::default()),
            disk: Some(DiskMetrics::default()),
            ..Metrics::default()
        };
        let bound = bind(&metrics, &[ComponentKind::Network, ComponentKind::Disk]);
        assert_eq!(bound[0].1.primary(), "--");
        assert_eq!(bound[0].1.secondary(), "");
        assert_eq!(bound[1].1.primary(), "--");
        assert_eq!(bound[1].1.secondary(), "");
    }

    #[test]
    fn byte_formatting_uses_binary_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_bytes(1024u64.pow(4)), "1.0 TB");
    }

    #[test]
    fn rate_formatting_adds_per_second() {
        assert_eq!(format_rate(0), "0 B/s");
        assert_eq!(format_rate(1024), "1.0 KB/s");
        assert_eq!(format_rate(1024 * 1024), "1.0 MB/s");
        assert_eq!(format_rate(1024 * 1024 * 1024), "1.0 GB/s");
    }

    #[test]
    fn percentage_is_clamped_and_rounded() {
        assert_eq!(format_pct(Some(42.4)), "42%");
        assert_eq!(format_pct(Some(42.6)), "43%");
        assert_eq!(format_pct(Some(-1.0)), "0%");
        assert_eq!(format_pct(Some(120.0)), "100%");
        assert_eq!(format_pct(Some(f32::NAN)), "--");
        assert_eq!(format_pct(None), "--");
    }

    #[test]
    fn temperature_uses_half_width_c() {
        assert_eq!(format_temp(Some(45.4)), "45°C");
        assert_eq!(format_temp(None), "--");
        assert_eq!(format_temp(Some(f32::INFINITY)), "--");
    }

    #[test]
    fn default_labels_are_chinese() {
        assert_eq!(default_label(ComponentKind::Memory), "内存");
        assert_eq!(default_label(ComponentKind::Network), "网络");
        assert_eq!(default_label(ComponentKind::Disk), "磁盘");
    }
}
