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
            let wave = |a: f64, b: f64, p: f64| a + (b - a) * (0.5 + 0.5 * (t / p).sin());
            let snap = Snapshot {
                timestamp: 0,
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
                        status: Some("active".into()),
                        temp: Some(41),
                        read_bytes_per_sec: Some((wave(0.0, 40.0, 5.0) * 1e6) as u64),
                        write_bytes_per_sec: Some((wave(0.0, 8.0, 6.0) * 1e6) as u64),
                    },
                    Disk {
                        device: "/dev/sdb".into(),
                        status: Some("standby".into()),
                        temp: None,
                        read_bytes_per_sec: Some(0),
                        write_bytes_per_sec: Some(0),
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
                    rx_bytes_per_sec: Some((wave(0.5, 60.0, 6.5) * 1e6) as u64),
                    tx_bytes_per_sec: Some((wave(0.1, 10.0, 7.5) * 1e6) as u64),
                }]),
            };
            {
                let mut st = cx.state.write().unwrap();
                st.conn = ConnStatus::Online;
                st.snap = snap;
                st.has_data = true;
                st.bump_seq();
            }
            post_update(cx.hwnd);
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}
