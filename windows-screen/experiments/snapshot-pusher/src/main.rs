//! T5 真机回归推送模拟器：模拟主客户端经命名管道推送快照（任务书 32 §4.4）。
//!
//! 仅用于真机验收，不含 NAS 地址/Token，不做任何网络连接。
//! 使用前需在小屏配置 `[integration].client_exe = "snapshot-pusher.exe"`，
//! 否则对端身份校验会拒绝连接（这是预期安全行为）。
//!
//! 用法：
//!   snapshot-pusher [--pipe NAME] [--count N] [--interval-ms MS] [--scenario demo|missing]
//!     --pipe        管道名（默认 hetao-screen，与 [integration].pipe 对齐）
//!     --count       推送条数（默认 0 = 无限，Ctrl+C 结束）
//!     --interval-ms 推送间隔（默认 1000，与 display.refresh_secs 对齐）
//!     --scenario    demo=全指标渐变（R1/R2）；missing=cpu.usage_pct=None 且仅 CPU/内存（验证 `--` 与整卡隐藏）

use std::io::Write;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use screen_core::{
    CpuMetrics, DiskMetrics, GpuMetrics, MemoryMetrics, NetworkMetrics, Snapshot,
};
use screen_ipc::{normalize_pipe_name, Message, MessageKind};

fn epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 确定性三角波：0..1 往返，period 为单程步数。
fn tri(i: u64, period: u64) -> f32 {
    let period = period.max(1);
    let phase = (i % (2 * period)) as f32;
    if phase <= period as f32 {
        phase / period as f32
    } else {
        (2.0 * period as f32 - phase) / period as f32
    }
}

fn lerp(t: f32, min: f32, max: f32) -> f32 {
    min + (max - min) * t
}

/// demo 场景：五项指标全量、数值持续渐变，便于肉眼确认屏幕在刷新。
fn demo_snapshot(seq: u64) -> Snapshot {
    let mut snap = Snapshot::new(seq, epoch_ms());
    let cpu = lerp(tri(seq, 30), 20.0, 80.0);
    let mem_pct = lerp(tri(seq, 45), 45.0, 75.0);
    let total: u64 = 16 * 1024 * 1024 * 1024;
    snap.metrics.cpu = Some(CpuMetrics {
        usage_pct: Some(cpu),
        temp_c: Some(lerp(tri(seq, 60), 40.0, 65.0)),
        ..Default::default()
    });
    snap.metrics.memory = Some(MemoryMetrics {
        usage_pct: Some(mem_pct),
        used_bytes: Some((mem_pct / 100.0 * total as f32) as u64),
        total_bytes: Some(total),
        ..Default::default()
    });
    snap.metrics.gpu = Some(GpuMetrics {
        name: Some("Demo GPU".to_string()),
        usage_pct: Some(lerp(tri(seq, 25), 10.0, 90.0)),
        temp_c: Some(lerp(tri(seq, 50), 42.0, 70.0)),
        ..Default::default()
    });
    snap.metrics.network = Some(NetworkMetrics {
        iface: Some("以太网".to_string()),
        up_bps: Some((lerp(tri(seq, 20), 0.0, 50.0) * 1024.0 * 1024.0) as u64),
        down_bps: Some((lerp(tri(seq, 15), 0.0, 80.0) * 1024.0 * 1024.0) as u64),
    });
    snap.metrics.disk = Some(DiskMetrics {
        device: Some("C:".to_string()),
        read_bps: Some((lerp(tri(seq, 18), 0.0, 200.0) * 1024.0 * 1024.0) as u64),
        write_bps: Some((lerp(tri(seq, 22), 0.0, 120.0) * 1024.0 * 1024.0) as u64),
    });
    snap
}

/// missing 场景：`usage_pct=None` 必须显示 `--`；GPU/网络/磁盘整卡隐藏。
fn missing_snapshot(seq: u64) -> Snapshot {
    let mut snap = Snapshot::new(seq, epoch_ms());
    snap.metrics.cpu = Some(CpuMetrics {
        usage_pct: None,
        temp_c: None,
        ..Default::default()
    });
    let total: u64 = 16 * 1024 * 1024 * 1024;
    snap.metrics.memory = Some(MemoryMetrics {
        usage_pct: Some(62.0),
        used_bytes: Some(10 * 1024 * 1024 * 1024),
        total_bytes: Some(total),
        ..Default::default()
    });
    snap
}

struct Options {
    pipe: String,
    count: u64,
    interval_ms: u64,
    scenario: String,
}

fn parse_args() -> Result<Options, String> {
    let mut opts = Options {
        pipe: "hetao-screen".to_string(),
        count: 0,
        interval_ms: 1_000,
        scenario: "demo".to_string(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut take = |name: &str| -> Result<String, String> {
            args.next()
                .ok_or_else(|| format!("{name} 缺少参数值"))
        };
        match arg.as_str() {
            "--pipe" => opts.pipe = take("--pipe")?,
            "--count" => {
                let v = take("--count")?;
                opts.count = v
                    .parse()
                    .map_err(|_| format!("--count 非法：{v}"))?;
            }
            "--interval-ms" => {
                let v = take("--interval-ms")?;
                opts.interval_ms = v
                    .parse()
                    .map_err(|_| format!("--interval-ms 非法：{v}"))?;
            }
            "--scenario" => opts.scenario = take("--scenario")?,
            "--help" | "-h" => {
                println!(
                    "snapshot-pusher [--pipe NAME] [--count N] [--interval-ms MS] [--scenario demo|missing]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("未知参数：{other}")),
        }
    }
    if opts.interval_ms < 100 {
        return Err("--interval-ms 不得小于 100".to_string());
    }
    if !["demo", "missing"].contains(&opts.scenario.as_str()) {
        return Err(format!("--scenario 只支持 demo|missing：{}", opts.scenario));
    }
    Ok(opts)
}

fn main() -> Result<(), String> {
    let opts = parse_args()?;
    let path = normalize_pipe_name(&opts.pipe);
    let mut pipe = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .map_err(|e| format!("打开管道失败 {path}：{e}（小屏进程是否在运行？）"))?;
    println!("connected: {path}");

    let mut write_msg = |msg: &Message, label: &str| -> Result<(), String> {
        let bytes = msg.to_bytes().map_err(|e| e.to_string())?;
        pipe.write_all(&bytes)
            .and_then(|()| pipe.flush())
            .map_err(|e| format!("写 {label} 失败：{e}"))?;
        println!("{label}: seq={} bytes={}", msg.header.seq, bytes.len());
        Ok(())
    };

    let hello = Message::new(MessageKind::Hello, 0, Vec::new()).map_err(|e| e.to_string())?;
    write_msg(&hello, "hello")?;

    let mut seq: u64 = 1;
    loop {
        if opts.count != 0 && seq > opts.count {
            break;
        }
        let snap = match opts.scenario.as_str() {
            "demo" => demo_snapshot(seq),
            _ => missing_snapshot(seq),
        };
        let msg = Message::with_payload(MessageKind::Snapshot, seq, &snap)
            .map_err(|e| e.to_string())?;
        write_msg(&msg, "snapshot")?;
        seq += 1;
        sleep(Duration::from_millis(opts.interval_ms));
    }

    let bye = Message::new(MessageKind::Bye, seq, Vec::new()).map_err(|e| e.to_string())?;
    write_msg(&bye, "bye")?;
    println!("done: sent {} snapshots", seq - 1);
    Ok(())
}
