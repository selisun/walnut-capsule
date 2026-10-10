//! FakeSource：无 NAS 时生成模拟数据，用于界面开发与测试。

use super::*;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

pub struct FakeSource;

impl DataSource for FakeSource {
    fn run(self, cx: SourceCtx) {
        let t0 = Instant::now();
        loop {
            match cx.cmd_rx.try_recv() {
                Ok(SourceCmd::Shutdown) => return,
                Ok(_) => {}
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => return,
            }
            let t = t0.elapsed().as_secs_f64();
            // 突发型流量：大部分时间贴底，周期性冲高回落，模拟真实网络/磁盘的心电图尖峰。
            // burst(t, peak, period, duty) = 在每个周期内 duty 比例的时间为 peak，其余为 0。
            let burst = |peak: f64, period: f64, duty: f64, phase: f64| {
                let phase_t = (t + phase) % period;
                if phase_t < period * duty {
                    // 冲高段：用半正弦让尖顶更自然
                    let k = (phase_t / (period * duty) * std::f64::consts::PI).sin();
                    peak * k
                } else {
                    0.0
                }
            };
            let wave = |a: f64, b: f64, p: f64| a + (b - a) * (0.5 + 0.5 * (t / p).sin());
            let snap = Snapshot {
                timestamp: 0,
                hostname: Some("hetao-nas".into()),
                cpu: Some(Cpu {
                    usage: Some(wave(10.0, 70.0, 7.0)),
                    temp: Some(wave(38.0, 55.0, 11.0)),
                }),
                memory: Some(Memory {
                    usage: Some(wave(40.0, 80.0, 13.0)),
                    used: Some(9_831_000_000),
                    total: Some(16_000_000_000),
                }),
                disks: Some(vec![
                    Disk {
                        device: "/dev/sda".into(),
                        read_bytes_per_sec: Some((burst(40.0e6, 5.0, 0.18, 0.0)) as u64),
                        write_bytes_per_sec: Some((burst(8.0e6, 6.0, 0.15, 2.0)) as u64),
                        used_bytes: Some(2_530_000_000_000),
                        total_bytes: Some(4_000_000_000_000),
                    },
                    Disk {
                        device: "/dev/sdb".into(),
                        read_bytes_per_sec: Some(0),
                        write_bytes_per_sec: Some(0),
                        used_bytes: Some(6_710_000_000_000),
                        total_bytes: Some(8_000_000_000_000),
                    },
                    Disk {
                        device: "/dev/sdc".into(),
                        read_bytes_per_sec: Some((burst(12.0e6, 4.5, 0.16, 1.0)) as u64),
                        write_bytes_per_sec: Some((burst(5.0e6, 8.0, 0.12, 3.0)) as u64),
                        used_bytes: Some(1_210_000_000_000),
                        total_bytes: Some(2_000_000_000_000),
                    },
                ]),
                // 4 卷：vol2 跨 sda+sdb 多盘，用于验收卷展开；sdb standby 展开后灰显
                volumes: Some(vec![
                    Volume {
                        name: "vol1".into(),
                        used_bytes: Some(5_800_000_000),
                        total_bytes: Some(25_000_000_000),
                    },
                    Volume {
                        name: "vol2".into(),
                        used_bytes: Some(4_500_000_000_000),
                        total_bytes: Some(8_000_000_000_000),
                    },
                    Volume {
                        name: "vol4".into(),
                        used_bytes: Some(1_900_000_000_000),
                        total_bytes: Some(8_000_000_000_000),
                    },
                    Volume {
                        name: "vol5".into(),
                        used_bytes: Some(896_000_000_000),
                        total_bytes: Some(2_000_000_000_000),
                    },
                ]),
                gpus: Some(vec![Gpu {
                    index: 0,
                    name: "NVIDIA RTX 4090".into(),
                    temp: Some(wave(45.0, 70.0, 9.0) as i64),
                    utilization: Some(wave(5.0, 95.0, 8.0) as i32),
                    memory_used: Some(18_432_000_000),
                    memory_total: Some(24_576_000_000),
                }]),
                network: Some(vec![NetIf {
                    name: "eth0".into(),
                    rx_bytes_per_sec: Some((burst(60.0e6, 6.5, 0.20, 0.5)) as u64),
                    tx_bytes_per_sec: Some((burst(10.0e6, 7.5, 0.14, 3.5)) as u64),
                }]),
            };
            {
                let mut st = cx.state.write().unwrap();
                st.conn = ConnStatus::Online;
                st.snap = Arc::new(snap);
                st.has_data = true;
                st.bump_seq();
            }
            post_update(cx.hwnd);
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}
