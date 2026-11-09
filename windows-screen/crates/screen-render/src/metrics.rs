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
    /// 逻辑卷（NAS 的 storage[]）。单卷时显示卷名；多卷且容量齐全时按合计显示。
    Storage {
        name: Option<String>,
        used: Option<u64>,
        total: Option<u64>,
    },
    /// 温度：CPU / GPU 中至少一个存在；主数值优先 CPU。
    Temp { cpu: Option<f32>, gpu: Option<f32> },
    /// 本地时钟（不依赖胶囊数据）。
    Clock {
        hour: u8,
        minute: u8,
        month: u8,
        day: u8,
    },
}

/// 本地时间的显示分量。由平台层取本地时间，格式化保持纯函数便于测试。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockParts {
    pub hour: u8,
    pub minute: u8,
    pub month: u8,
    pub day: u8,
}

impl MetricView {
    pub fn kind(&self) -> ComponentKind {
        match self {
            MetricView::Cpu { .. } => ComponentKind::Cpu,
            MetricView::Memory { .. } => ComponentKind::Memory,
            MetricView::Gpu { .. } => ComponentKind::Gpu,
            MetricView::Network { .. } => ComponentKind::Network,
            MetricView::Disk { .. } => ComponentKind::Disk,
            MetricView::Storage { .. } => ComponentKind::Storage,
            MetricView::Temp { .. } => ComponentKind::Temp,
            MetricView::Clock { .. } => ComponentKind::Clock,
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
            MetricView::Storage { used, total, .. } => format_pct(pct_of(*used, *total)),
            MetricView::Temp { cpu, gpu } => format_temp(cpu.or(*gpu)),
            MetricView::Clock { hour, minute, .. } => format!("{:02}:{:02}", hour, minute),
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
            MetricView::Storage { name, used, total } => {
                let pair = format_pair(*used, *total);
                match name.as_deref().filter(|n| !n.is_empty()) {
                    Some(name) => format!("{name} {pair}"),
                    None => pair,
                }
            }
            MetricView::Temp { cpu, gpu } => {
                // 主数值已用 CPU（缺则用 GPU）；副信息补另一个来源，避免重复
                if cpu.is_some() && gpu.is_some() {
                    format!("显卡 {}", format_temp(*gpu))
                } else {
                    String::new()
                }
            }
            MetricView::Clock { month, day, .. } => format!("{:02}-{:02}", month, day),
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
    bind_with_clock(metrics, visible, None)
}

/// 与 `bind` 相同，但可注入本地时间以显示时钟卡（`None` 时忽略 `Clock`）。
pub fn bind_with_clock(
    metrics: &Metrics,
    visible: &[ComponentKind],
    clock: Option<ClockParts>,
) -> Vec<(ComponentKind, MetricView)> {
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
            ComponentKind::Storage => storage_view(metrics),
            ComponentKind::Temp => temp_view(metrics),
            ComponentKind::Clock => clock.map(|c| MetricView::Clock {
                hour: c.hour,
                minute: c.minute,
                month: c.month,
                day: c.day,
            }),
        };
        if let Some(view) = view {
            out.push((*kind, view));
        }
    }
    out
}

/// 逻辑卷视图：单卷用自身数值；多卷且容量齐全时按合计；否则回退首卷。
fn storage_view(metrics: &Metrics) -> Option<MetricView> {
    let first = metrics.storage.first()?;
    if metrics.storage.len() == 1 {
        return Some(MetricView::Storage {
            name: non_empty(&first.name),
            used: first.used_bytes,
            total: first.total_bytes,
        });
    }
    let all_complete = metrics
        .storage
        .iter()
        .all(|v| v.used_bytes.is_some() && v.total_bytes.is_some());
    if all_complete {
        let used = metrics
            .storage
            .iter()
            .fold(0u64, |acc, v| acc.saturating_add(v.used_bytes.unwrap_or(0)));
        let total = metrics.storage.iter().fold(0u64, |acc, v| {
            acc.saturating_add(v.total_bytes.unwrap_or(0))
        });
        Some(MetricView::Storage {
            name: None,
            used: Some(used),
            total: Some(total),
        })
    } else {
        Some(MetricView::Storage {
            name: non_empty(&first.name),
            used: first.used_bytes,
            total: first.total_bytes,
        })
    }
}

fn non_empty(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn pct_of(used: Option<u64>, total: Option<u64>) -> Option<f32> {
    match (used, total) {
        (Some(u), Some(t)) if t > 0 => Some(((u as f64 / t as f64) * 100.0) as f32),
        _ => None,
    }
}

/// 温度视图：CPU / GPU 至少一个存在才出卡。
fn temp_view(metrics: &Metrics) -> Option<MetricView> {
    let cpu = metrics.cpu.as_ref().and_then(|c| c.temp_c);
    let gpu = metrics.gpu.as_ref().and_then(|g| g.temp_c);
    if cpu.is_none() && gpu.is_none() {
        return None;
    }
    Some(MetricView::Temp { cpu, gpu })
}

/// 本地时间（Windows）。取不到时返回 `None`，时钟卡不显示，绝不回退到 UTC 硬编码。
#[cfg(windows)]
pub fn local_clock() -> Option<ClockParts> {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let now = unsafe { GetLocalTime() };
    Some(ClockParts {
        hour: now.wHour as u8,
        minute: now.wMinute as u8,
        month: now.wMonth as u8,
        day: now.wDay as u8,
    })
}

#[cfg(not(windows))]
pub fn local_clock() -> Option<ClockParts> {
    None
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
    use screen_core::{
        CpuMetrics, DiskMetrics, GpuMetrics, MemoryMetrics, NetworkMetrics, StorageVolume,
    };

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
                vendor: None,
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
        assert_eq!(default_label(ComponentKind::Storage), "存储");
    }

    fn volume(name: &str, used: Option<u64>, total: Option<u64>) -> StorageVolume {
        StorageVolume {
            name: name.to_string(),
            used_bytes: used,
            total_bytes: total,
        }
    }

    #[test]
    fn storage_hidden_when_absent() {
        let metrics = Metrics::default();
        assert!(bind(&metrics, &[ComponentKind::Storage]).is_empty());
    }

    #[test]
    fn single_volume_shows_name_and_capacity() {
        let metrics = Metrics {
            storage: vec![volume(
                "vol1",
                Some(9 * 1024 * 1024 * 1024),
                Some(20 * 1024 * 1024 * 1024),
            )],
            ..Metrics::default()
        };
        let bound = bind(&metrics, &[ComponentKind::Storage]);
        assert_eq!(bound.len(), 1);
        let view = &bound[0].1;
        assert_eq!(view.kind(), ComponentKind::Storage);
        assert_eq!(view.primary(), "45%");
        assert_eq!(view.secondary(), "vol1 9.0 GB / 20.0 GB");
    }

    #[test]
    fn multiple_complete_volumes_are_summed() {
        let metrics = Metrics {
            storage: vec![
                volume(
                    "vol1",
                    Some(3 * 1024 * 1024 * 1024),
                    Some(10 * 1024 * 1024 * 1024),
                ),
                volume(
                    "vol2",
                    Some(7 * 1024 * 1024 * 1024),
                    Some(10 * 1024 * 1024 * 1024),
                ),
            ],
            ..Metrics::default()
        };
        let bound = bind(&metrics, &[ComponentKind::Storage]);
        let view = &bound[0].1;
        assert_eq!(view.primary(), "50%");
        assert_eq!(view.secondary(), "10.0 GB / 20.0 GB");
    }

    #[test]
    fn incomplete_multiple_volumes_fall_back_to_first() {
        let metrics = Metrics {
            storage: vec![
                volume("vol1", Some(1_000_000_000), Some(2_000_000_000)),
                volume("vol2", None, None),
            ],
            ..Metrics::default()
        };
        let bound = bind(&metrics, &[ComponentKind::Storage]);
        let view = &bound[0].1;
        assert_eq!(view.primary(), "50%");
        assert!(view.secondary().starts_with("vol1 "));
    }

    #[test]
    fn storage_zero_total_shows_dash_not_zero_percent() {
        let metrics = Metrics {
            storage: vec![volume("vol1", Some(0), Some(0))],
            ..Metrics::default()
        };
        let bound = bind(&metrics, &[ComponentKind::Storage]);
        let view = &bound[0].1;
        assert_eq!(view.primary(), "--");
        assert_eq!(view.secondary(), "vol1 0 B / 0 B");
    }

    #[test]
    fn storage_without_capacity_shows_dash() {
        let metrics = Metrics {
            storage: vec![volume("vol1", None, Some(1_000))],
            ..Metrics::default()
        };
        let bound = bind(&metrics, &[ComponentKind::Storage]);
        let view = &bound[0].1;
        assert_eq!(view.primary(), "--");
        assert_eq!(view.secondary(), "vol1 -- / 1000 B");
    }

    fn cpu_with_temp(temp: Option<f32>) -> CpuMetrics {
        CpuMetrics {
            temp_c: temp,
            ..CpuMetrics::default()
        }
    }

    fn gpu_with_temp(temp: Option<f32>) -> GpuMetrics {
        GpuMetrics {
            temp_c: temp,
            ..GpuMetrics::default()
        }
    }

    #[test]
    fn temp_hidden_when_no_sensor() {
        let metrics = Metrics {
            cpu: Some(cpu_with_temp(None)),
            gpu: Some(gpu_with_temp(None)),
            ..Metrics::default()
        };
        assert!(bind(&metrics, &[ComponentKind::Temp]).is_empty());
        assert!(bind(&Metrics::default(), &[ComponentKind::Temp]).is_empty());
    }

    #[test]
    fn temp_cpu_only_shows_cpu_reading() {
        let metrics = Metrics {
            cpu: Some(cpu_with_temp(Some(45.0))),
            ..Metrics::default()
        };
        let bound = bind(&metrics, &[ComponentKind::Temp]);
        let view = &bound[0].1;
        assert_eq!(view.kind(), ComponentKind::Temp);
        assert_eq!(view.primary(), "45°C");
        assert_eq!(view.secondary(), "");
    }

    #[test]
    fn temp_gpu_only_falls_back_to_gpu_reading() {
        let metrics = Metrics {
            gpu: Some(gpu_with_temp(Some(60.0))),
            ..Metrics::default()
        };
        let bound = bind(&metrics, &[ComponentKind::Temp]);
        let view = &bound[0].1;
        assert_eq!(view.primary(), "60°C");
        assert_eq!(view.secondary(), "");
    }

    #[test]
    fn temp_both_sensors_show_gpu_in_secondary() {
        let metrics = Metrics {
            cpu: Some(cpu_with_temp(Some(45.0))),
            gpu: Some(gpu_with_temp(Some(60.0))),
            ..Metrics::default()
        };
        let bound = bind(&metrics, &[ComponentKind::Temp]);
        let view = &bound[0].1;
        assert_eq!(view.primary(), "45°C");
        assert_eq!(view.secondary(), "显卡 60°C");
    }

    fn clock(hour: u8, minute: u8, month: u8, day: u8) -> ClockParts {
        ClockParts {
            hour,
            minute,
            month,
            day,
        }
    }

    #[test]
    fn clock_formats_time_and_date() {
        let parts = clock(9, 5, 11, 4);
        let view = MetricView::Clock {
            hour: parts.hour,
            minute: parts.minute,
            month: parts.month,
            day: parts.day,
        };
        assert_eq!(view.primary(), "09:05");
        assert_eq!(view.secondary(), "11-04");
    }

    #[test]
    fn clock_hidden_without_local_time() {
        let metrics = Metrics::default();
        assert!(bind(&metrics, &[ComponentKind::Clock]).is_empty());
        assert!(bind_with_clock(&metrics, &[ComponentKind::Clock], None).is_empty());
    }

    #[test]
    fn bind_with_clock_respects_visible_order_and_metrics() {
        let metrics = Metrics {
            cpu: Some(cpu_with_temp(Some(45.0))),
            ..Metrics::default()
        };
        let parts = clock(23, 59, 12, 31);
        let bound = bind_with_clock(
            &metrics,
            &[
                ComponentKind::Clock,
                ComponentKind::Cpu,
                ComponentKind::Temp,
            ],
            Some(parts),
        );
        assert_eq!(
            bound.iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
            vec![
                ComponentKind::Clock,
                ComponentKind::Cpu,
                ComponentKind::Temp
            ]
        );
        assert_eq!(bound[0].1.primary(), "23:59");
        assert_eq!(bound[0].1.secondary(), "12-31");
        assert_eq!(bound[2].1.primary(), "45°C");
    }

    #[test]
    fn temp_and_clock_labels_are_chinese() {
        assert_eq!(default_label(ComponentKind::Temp), "温度");
        assert_eq!(default_label(ComponentKind::Clock), "时钟");
    }
}
