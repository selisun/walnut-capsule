//! B-10 快照推送桥（P1 阶段）：把 capsule 的 NAS 指标映射为 `screen-ipc` 快照并编码。
//!
//! 本阶段**只做映射与编码**，不连接管道（P2 再接）。因此本模块对运行时无副作用，
//! 由 `main.rs` 以 `#[allow(dead_code)]` 引入，待 P2 使用。
//!
//! 协议基线（与 `windows-screen/crates/screen-ipc/src/header.rs` 一致，不得擅自更改）：
//!
//! ```text
//! magic(4) | major(2) | minor(2) | kind(1) | flags(1) | seq(8) | payload_len(4)
//! ```
//!
//! 总计 22 字节，小端序；`magic = "HTSP"`，`major = 1`，单帧上限 256 KiB。
//!
//! **禁止写入 0 冒充缺失值**：上游 `None` 一律省略字段，由小屏渲染成 `--`。

use serde::Serialize;

use crate::data;

pub const MAGIC: [u8; 4] = *b"HTSP";
pub const HEADER_LEN: usize = 22;
pub const SCHEMA_MAJOR: u16 = 1;
pub const SCHEMA_MINOR: u16 = 0;
pub const MAX_FRAME_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Hello = 1,
    Snapshot = 3,
    Bye = 6,
}

impl MessageKind {
    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutNas {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub online: bool,
    /// 顶栏显示的地址（host 或 host:port），不含凭据。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
}

/// 电脑本机指标（双显主题右列）。
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct OutPc {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_temp: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct OutCpu {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_1m: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_5m: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_15m: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_30m: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct OutMemory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct OutGpu {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vram_used_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vram_total_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct OutNetwork {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub down_bps: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct OutDisk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_bps: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct OutVolume {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct OutMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<OutCpu>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<OutMemory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<OutGpu>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<OutNetwork>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk: Option<OutDisk>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub storage: Vec<OutVolume>,
}

/// 推给小屏的快照（字段顺序与 `screen-core::Snapshot` 保持一致，便于逐字节比对）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScreenSnapshot {
    pub seq: u64,
    pub ts_ms: i64,
    pub nas: OutNas,
    pub metrics: OutMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pc: Option<OutPc>,
}

/// 把 capsule 的 NAS 指标映射为小屏快照。
///
/// 多盘/多卡/多网卡的取值规则（提案 `37` §4.1，固定不可变）：
/// - 磁盘：按 `device` 字典序排序后取第一块；
/// - 显卡：取 `index` 最小的一块；
/// - 网卡：取 `network[0]`（保持上游顺序）。
///
/// `ts_ms` 一律取**胶囊采样时刻**（本机时钟），**不采用 NAS 端时间戳**：
/// `screen-core` 定义该字段为「客户端采样时间」，且小屏用过它和本机时钟比较。
/// 实测（P2 真机）NAS 侧时间戳与本机存在偏差，直接透传会让每帧都判「数据过期」。
/// 上游断开时胶囊本身停止推送，故不会把旧数据当实时。
pub fn map_snapshot(src: &data::Snapshot, online: bool, seq: u64, now_ms: i64) -> ScreenSnapshot {
    map_snapshot_with(src, online, seq, now_ms, None, None)
}

/// 同 `map_snapshot`，额外带上电脑本机指标与 NAS 显示地址。
pub fn map_snapshot_with(
    src: &data::Snapshot,
    online: bool,
    seq: u64,
    now_ms: i64,
    pc: Option<OutPc>,
    nas_addr: Option<&str>,
) -> ScreenSnapshot {
    let ts_ms = now_ms;

    let cpu = src.cpu.as_ref().map(|c| OutCpu {
        usage_pct: c.usage.map(|v| v as f32),
        temp_c: c.temp.map(|v| v as f32),
        load_1m: c.load1.map(|v| v as f32),
        load_5m: c.load5.map(|v| v as f32),
        load_15m: c.load15.map(|v| v as f32),
        load_30m: c.load30.map(|v| v as f32),
    });

    let memory = src.memory.as_ref().map(|m| OutMemory {
        usage_pct: m.usage.map(|v| v as f32),
        used_bytes: m.used,
        total_bytes: m.total,
        cached_bytes: m.cache,
        available_bytes: m.free,
    });

    let gpu = pick_gpu(src).map(|g| OutGpu {
        name: if g.name.is_empty() {
            None
        } else {
            Some(g.name.clone())
        },
        usage_pct: g.utilization.map(|v| v as f32),
        temp_c: g.temp.map(|v| v as f32),
        vram_used_bytes: g.memory_used,
        vram_total_bytes: g.memory_total,
    });

    let network = src
        .network
        .as_ref()
        .and_then(|list| list.first())
        .map(|n| OutNetwork {
            iface: if n.name.is_empty() {
                None
            } else {
                Some(n.name.clone())
            },
            up_bps: n.tx_bytes_per_sec,
            down_bps: n.rx_bytes_per_sec,
        });

    let disk = pick_disk(src).map(|d| OutDisk {
        device: if d.device.is_empty() {
            None
        } else {
            Some(d.device.clone())
        },
        read_bps: d.read_bytes_per_sec,
        write_bps: d.write_bytes_per_sec,
    });

    let storage = src
        .volumes
        .as_ref()
        .map(|list| {
            list.iter()
                .map(|v| OutVolume {
                    name: v.name.clone(),
                    used_bytes: v.used_bytes,
                    total_bytes: v.total_bytes,
                })
                .collect()
        })
        .unwrap_or_default();

    ScreenSnapshot {
        seq,
        ts_ms,
        nas: OutNas {
            name: src.hostname.clone(),
            online,
            addr: nas_addr.filter(|s| !s.is_empty()).map(|s| s.to_string()),
        },
        metrics: OutMetrics {
            cpu,
            memory,
            gpu,
            network,
            disk,
            storage,
        },
        pc,
    }
}

fn pick_disk(src: &data::Snapshot) -> Option<&data::Disk> {
    let disks = src.disks.as_ref()?;
    disks.iter().min_by(|a, b| a.device.cmp(&b.device))
}

fn pick_gpu(src: &data::Snapshot) -> Option<&data::Gpu> {
    let gpus = src.gpus.as_ref()?;
    gpus.iter().min_by_key(|g| g.index)
}

impl ScreenSnapshot {
    /// 序列化为小屏可解析的 JSON（缺失字段被省略，绝不以 0 填充）。
    pub fn payload_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| format!("快照序列化失败：{e}"))
    }
}

/// 编码为完整消息（22 字节头 + JSON）。`payload_len` 超上限时拒绝。
pub fn encode(kind: MessageKind, seq: u64, payload: &[u8]) -> Result<Vec<u8>, String> {
    let payload_len = payload.len();
    if payload_len + HEADER_LEN > MAX_FRAME_BYTES {
        return Err(format!(
            "payload 过大：{}（上限 {}）",
            payload_len,
            MAX_FRAME_BYTES - HEADER_LEN
        ));
    }
    let mut out = Vec::with_capacity(HEADER_LEN + payload_len);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&SCHEMA_MAJOR.to_le_bytes());
    out.extend_from_slice(&SCHEMA_MINOR.to_le_bytes());
    out.push(kind.to_u8());
    out.push(0); // flags
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&(payload_len as u32).to_le_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// 便捷：编码一条快照消息。
pub fn encode_snapshot(snapshot: &ScreenSnapshot) -> Result<Vec<u8>, String> {
    let payload = snapshot.payload_json()?;
    encode(MessageKind::Snapshot, snapshot.seq, payload.as_bytes())
}

// ---------------------------------------------------------------------------
// P2：管道客户端与后台推送循环
// ---------------------------------------------------------------------------

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 与 `screen-app` 的默认管道名一致（`\\.\pipe\hetao-screen`）。
pub const PIPE_PATH: &str = r"\\.\pipe\hetao-screen";

/// 管道写入抽象：真实实现是命名管道文件句柄；测试用内存实现。
pub trait PipeSink: Send {
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()>;
}

struct FileSink {
    file: std::fs::File,
}

impl PipeSink for FileSink {
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.file.write_all(bytes)?;
        self.file.flush()
    }
}

/// 打开管道；小屏未运行时返回 `None`（不视为错误，仅下个周期重试）。
fn open_pipe() -> Option<Box<dyn PipeSink>> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(PIPE_PATH)
        .ok()?;
    Some(Box::new(FileSink { file }))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PushStats {
    pub connects: u64,
    pub hellos: u64,
    pub snapshots: u64,
    pub byes: u64,
    pub write_errors: u64,
}

fn write_msg(sink: &mut Option<Box<dyn PipeSink>>, bytes: &[u8]) -> bool {
    match sink.as_mut() {
        Some(s) => s.write_all(bytes).is_ok(),
        None => false,
    }
}

/// 推送主循环。参数全部注入，便于无管道单测。
///
/// 行为约定（提案 `37` §4.3 / §7）：
/// - 连接成功先发 `Hello`（seq 0），小屏据此重置序列基线；
/// - 仅当 NAS 在线**且有数据**时推送 `Snapshot`，seq 单调递增；
/// - 未连接时每个周期最多尝试一次，不重试风暴；
/// - 任一次写失败即丢弃连接，下个周期重连（不阻塞 UI）；
/// - 停止时发 `Bye`（best-effort）。
pub fn run_push_loop(
    shared: &Arc<RwLock<data::SharedState>>,
    refresh_ms: u64,
    stop: &AtomicBool,
    open: &mut dyn FnMut() -> Option<Box<dyn PipeSink>>,
    now: &mut dyn FnMut() -> i64,
    sleep: &mut dyn FnMut(u64),
) -> PushStats {
    let mut stats = PushStats::default();
    let mut sink: Option<Box<dyn PipeSink>> = None;
    let mut seq: u64 = 0;
    let refresh_ms = refresh_ms.max(1);
    let mut sampler = crate::pc_metrics::PcSampler::new();

    loop {
        if stop.load(Ordering::SeqCst) {
            if sink.is_some() {
                let bye = encode(MessageKind::Bye, seq + 1, b"").expect("bye 编码");
                if write_msg(&mut sink, &bye) {
                    stats.byes += 1;
                }
            }
            break;
        }

        if sink.is_none() {
            sink = open();
            if sink.is_some() {
                let hello = encode(MessageKind::Hello, 0, b"").expect("hello 编码");
                if write_msg(&mut sink, &hello) {
                    stats.connects += 1;
                    stats.hellos += 1;
                } else {
                    stats.write_errors += 1;
                    sink = None;
                    sleep(refresh_ms);
                    continue;
                }
            } else {
                // 小屏未启动：静默跳过，下周期再试一次。
                sleep(refresh_ms);
                continue;
            }
        }

        let (snapshot, online, has_data, nas_host) = match shared.read() {
            Ok(guard) => {
                let online = guard.conn == data::ConnStatus::Online;
                (
                    guard.snap.clone(),
                    online,
                    guard.has_data,
                    guard.nas_host.clone(),
                )
            }
            Err(_) => (
                Arc::new(data::Snapshot::default()),
                false,
                false,
                String::new(),
            ),
        };

        if online && has_data {
            seq += 1;
            let sample = sampler.sample();
            let pc = (!sample.is_empty()).then_some(OutPc {
                cpu_pct: sample.cpu_pct,
                mem_pct: sample.mem_pct,
                gpu_pct: sample.gpu_pct,
                gpu_temp: sample.gpu_temp,
            });
            let mapped = map_snapshot_with(&snapshot, online, seq, now(), pc, Some(&nas_host));
            if let Ok(bytes) = encode_snapshot(&mapped) {
                if write_msg(&mut sink, &bytes) {
                    stats.snapshots += 1;
                    // 低频诊断：每 30 次推送记一行指标（无凭据），用于现场判断「数据是否在更新」。
                    if seq % 30 == 0 {
                        let cpu = mapped.metrics.cpu.as_ref().and_then(|c| c.usage_pct);
                        let mem = mapped.metrics.memory.as_ref().and_then(|m| m.usage_pct);
                        log(&format!(
                            "snap seq={seq} ts={} cpu={:?} mem={:?}",
                            mapped.ts_ms, cpu, mem
                        ));
                    }
                } else {
                    stats.write_errors += 1;
                    sink = None;
                }
            }
        }

        sleep(refresh_ms);
    }

    stats
}

// ---------------------------------------------------------------------------
// 生产入口（由 `app::run` 在 NAS 数据源启动后调用）
// ---------------------------------------------------------------------------

static PUSH_STOP: AtomicBool = AtomicBool::new(false);
static PUSH_STARTED: AtomicBool = AtomicBool::new(false);
static PUSH_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
/// 每轮推送后重新读取，设置里改刷新周期无需重启线程。
static PUSH_INTERVAL_MS: AtomicU64 = AtomicU64::new(1_000);

/// 推送线程是否在跑（供设置界面判断是否需要拉起）。
pub fn is_running() -> bool {
    PUSH_STARTED.load(Ordering::SeqCst)
}

/// 运行中修改刷新周期（秒）。线程下一轮生效，不重启、不阻塞 UI。
pub fn set_interval_secs(refresh_secs: u32) {
    PUSH_INTERVAL_MS.store(u64::from(refresh_secs.max(1)) * 1_000, Ordering::SeqCst);
}

/// 启动后台推送线程（幂等）。失败只写日志，不影响客户端主流程。
pub fn start(shared: Arc<RwLock<data::SharedState>>, refresh_secs: u32) {
    set_interval_secs(refresh_secs);
    if PUSH_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    PUSH_STOP.store(false, Ordering::SeqCst);
    let refresh_ms = u64::from(refresh_secs.max(1)) * 1_000;
    let handle = std::thread::spawn(move || {
        let mut open = open_pipe;
        let mut now = epoch_ms;
        // 分片休眠：改周期即时生效，且 stop 最多 100ms 就能返回，不卡 UI。
        let mut sleep = |_ms: u64| {
            let mut left = PUSH_INTERVAL_MS.load(Ordering::SeqCst).max(1);
            while left > 0 {
                if PUSH_STOP.load(Ordering::SeqCst) {
                    return;
                }
                let step = left.min(100);
                std::thread::sleep(Duration::from_millis(step));
                left -= step;
            }
        };
        let stats = run_push_loop(
            &shared, refresh_ms, &PUSH_STOP, &mut open, &mut now, &mut sleep,
        );
        log(&format!(
            "push end: connects={} hellos={} snapshots={} byes={} write_errors={}",
            stats.connects, stats.hellos, stats.snapshots, stats.byes, stats.write_errors
        ));
    });
    match PUSH_THREAD.lock() {
        Ok(mut slot) => *slot = Some(handle),
        Err(_) => log("push thread 槽位锁中毒，已跳过登记"),
    }
}

/// 停止推送线程（分片休眠下最多约 100ms 返回），并让出锁避免阻塞退出。
pub fn stop() {
    if !PUSH_STARTED.swap(false, Ordering::SeqCst) {
        return;
    }
    PUSH_STOP.store(true, Ordering::SeqCst);
    let handle = PUSH_THREAD.lock().ok().and_then(|mut slot| slot.take());
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn interval_is_clamped_and_stored() {
        set_interval_secs(0);
        assert_eq!(PUSH_INTERVAL_MS.load(Ordering::SeqCst), 1_000);
        set_interval_secs(3);
        assert_eq!(PUSH_INTERVAL_MS.load(Ordering::SeqCst), 3_000);
        set_interval_secs(1);
    }

    #[test]
    fn push_thread_is_not_running_before_start() {
        assert!(!is_running());
    }
}

fn epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 追加到 `%APPDATA%\hetao-capsule\screen_bridge.log`（与 `screen_host.log` 同目录）。
fn log(msg: &str) {
    let Some(base) = std::env::var_os("APPDATA") else {
        return;
    };
    let dir = std::path::PathBuf::from(base).join("hetao-capsule");
    let _ = std::fs::create_dir_all(&dir);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("screen_bridge.log"))
        .and_then(|mut f| writeln!(f, "{}.{:03} {}", t.as_secs(), t.subsec_millis(), msg));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_source() -> data::Snapshot {
        data::Snapshot {
            timestamp: 1_700_000_000_000,
            hostname: Some("nas".to_string()),
            cpu: Some(data::Cpu {
                usage: Some(12.5),
                temp: Some(45.0),
                load1: Some(0.5),
                load5: Some(0.4),
                load15: Some(0.3),
                load30: None,
            }),
            memory: Some(data::Memory {
                usage: Some(55.0),
                used: Some(8_589_934_592),
                total: Some(17_179_869_184),
                cache: Some(2_147_483_648),
                free: Some(8_589_934_592),
            }),
            // 故意把 /dev/sdb 放前面，验证按 device 字典序取 /dev/sda
            disks: Some(vec![
                data::Disk {
                    device: "/dev/sdb".to_string(),
                    read_bytes_per_sec: Some(4096),
                    write_bytes_per_sec: Some(8192),
                    used_bytes: Some(3_000),
                    total_bytes: Some(4_000),
                },
                data::Disk {
                    device: "/dev/sda".to_string(),
                    read_bytes_per_sec: Some(1024),
                    write_bytes_per_sec: Some(2048),
                    used_bytes: Some(1_000),
                    total_bytes: Some(2_000),
                },
            ]),
            volumes: Some(vec![data::Volume {
                name: "vol1".to_string(),
                used_bytes: Some(1_000),
                total_bytes: Some(2_000),
            }]),
            // 故意把 index=1 放前面，验证取 index 最小的卡
            gpus: Some(vec![
                data::Gpu {
                    index: 1,
                    name: "card1".to_string(),
                    temp: Some(60),
                    utilization: Some(10),
                    memory_used: Some(0),
                    memory_total: Some(0),
                },
                data::Gpu {
                    index: 0,
                    name: "card0".to_string(),
                    temp: Some(50),
                    utilization: Some(30),
                    memory_used: Some(1_073_741_824),
                    memory_total: Some(8_589_934_592),
                },
            ]),
            network: Some(vec![data::NetIf {
                name: "eth0".to_string(),
                rx_bytes_per_sec: Some(100),
                tx_bytes_per_sec: Some(200),
                speed_mbps: Some(1000),
            }]),
            traffic: None,
        }
    }

    /// 与 `windows-screen/crates/screen-ipc/tests/b10_golden.rs` 中的常量**必须逐字节一致**。
    const GOLDEN_JSON: &str = "{\"seq\":42,\"ts_ms\":1700000000000,\"nas\":{\"name\":\"nas\",\"online\":true},\"metrics\":{\"cpu\":{\"usage_pct\":12.5,\"temp_c\":45.0,\"load_1m\":0.5,\"load_5m\":0.4,\"load_15m\":0.3},\"memory\":{\"usage_pct\":55.0,\"used_bytes\":8589934592,\"total_bytes\":17179869184,\"cached_bytes\":2147483648,\"available_bytes\":8589934592},\"gpu\":{\"name\":\"card0\",\"usage_pct\":30.0,\"temp_c\":50.0,\"vram_used_bytes\":1073741824,\"vram_total_bytes\":8589934592},\"network\":{\"iface\":\"eth0\",\"up_bps\":200,\"down_bps\":100},\"disk\":{\"device\":\"/dev/sda\",\"read_bps\":1024,\"write_bps\":2048},\"storage\":[{\"name\":\"vol1\",\"used_bytes\":1000,\"total_bytes\":2000}]}}";

    #[test]
    fn golden_json_is_stable_and_cross_checked() {
        let mapped = map_snapshot(&fixture_source(), true, 42, 1_700_000_000_000);
        assert_eq!(mapped.payload_json().expect("json"), GOLDEN_JSON);
    }

    #[test]
    fn header_layout_matches_screen_ipc() {
        let bytes = encode(MessageKind::Snapshot, 42, GOLDEN_JSON.as_bytes()).expect("encode");
        assert_eq!(&bytes[0..4], b"HTSP");
        assert_eq!(&bytes[4..6], &[1u8, 0]); // major = 1
        assert_eq!(&bytes[6..8], &[0u8, 0]); // minor = 0
        assert_eq!(bytes[8], 3); // Snapshot
        assert_eq!(bytes[9], 0); // flags
        assert_eq!(&bytes[10..18], &42u64.to_le_bytes());
        assert_eq!(&bytes[18..22], &(GOLDEN_JSON.len() as u32).to_le_bytes());
        assert_eq!(bytes.len(), HEADER_LEN + GOLDEN_JSON.len());
        assert_eq!(&bytes[HEADER_LEN..], GOLDEN_JSON.as_bytes());
    }

    #[test]
    fn missing_values_are_omitted_never_zero() {
        let src = data::Snapshot {
            timestamp: 1_700_000_000_000,
            hostname: None,
            cpu: Some(data::Cpu {
                usage: None,
                temp: None,
                load1: None,
                load5: None,
                load15: None,
                load30: None,
            }),
            ..Default::default()
        };
        let mapped = map_snapshot(&src, false, 1, 1_700_000_000_000);
        let json = mapped.payload_json().expect("json");
        assert!(!json.contains("usage_pct"), "缺失值不得写成 0：{json}");
        assert!(!json.contains("temp_c"));
        assert!(json.contains("\"online\":false"));
        assert!(!json.contains("\"gpu\""), "整项缺失应省略：{json}");
        assert!(!json.contains("\"disk\""));
        assert!(!json.contains("\"storage\""));
    }

    #[test]
    fn multi_disk_gpu_network_rules() {
        let mapped = map_snapshot(&fixture_source(), true, 42, 0);
        let disk = mapped.metrics.disk.expect("disk");
        assert_eq!(disk.device.as_deref(), Some("/dev/sda"));
        let gpu = mapped.metrics.gpu.expect("gpu");
        assert_eq!(gpu.name.as_deref(), Some("card0"));
        assert_eq!(gpu.usage_pct, Some(30.0));
        let net = mapped.metrics.network.expect("network");
        assert_eq!(net.iface.as_deref(), Some("eth0"));
        assert_eq!(net.up_bps, Some(200));
        assert_eq!(net.down_bps, Some(100));
    }

    #[test]
    fn ts_ms_is_client_sample_time_not_nas_clock() {
        // 回归：曾直接透传 NAS 时间戳，真机上每帧都被小屏判「数据过期」。
        let src = data::Snapshot {
            timestamp: 1_699_000_000, // 故意给一个与本机偏差很大的值（甚至像秒级时间戳）
            ..Default::default()
        };
        let mapped = map_snapshot(&src, false, 7, 1_700_000_123_456);
        assert_eq!(mapped.ts_ms, 1_700_000_123_456);
        assert_eq!(mapped.seq, 7);
    }

    #[test]
    fn empty_names_become_none() {
        let src = data::Snapshot {
            disks: Some(vec![data::Disk {
                device: String::new(),
                read_bytes_per_sec: None,
                write_bytes_per_sec: None,
                used_bytes: None,
                total_bytes: None,
            }]),
            ..Default::default()
        };
        let mapped = map_snapshot(&src, false, 1, 1);
        let json = mapped.payload_json().expect("json");
        assert!(
            !json.contains("\"device\":\"\""),
            "空设备名不得输出空串：{json}"
        );
    }

    #[test]
    fn hello_and_bye_have_empty_payload() {
        let hello = encode(MessageKind::Hello, 0, b"").expect("hello");
        assert_eq!(hello.len(), HEADER_LEN);
        assert_eq!(hello[8], 1);
        let bye = encode(MessageKind::Bye, 9, b"").expect("bye");
        assert_eq!(bye.len(), HEADER_LEN);
        assert_eq!(bye[8], 6);
        assert_eq!(&bye[10..18], &9u64.to_le_bytes());
    }

    #[test]
    fn oversized_payload_is_rejected() {
        let limit = MAX_FRAME_BYTES - HEADER_LEN;
        assert!(encode(MessageKind::Snapshot, 1, &vec![0u8; limit]).is_ok());
        assert!(encode(MessageKind::Snapshot, 1, &vec![0u8; limit + 1]).is_err());
    }

    // ---- P2 推送循环 ----

    struct MockSink {
        log: Arc<Mutex<Vec<Vec<u8>>>>,
        fail_after: Option<usize>,
        written: usize,
    }

    impl PipeSink for MockSink {
        fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
            if let Some(k) = self.fail_after {
                if self.written >= k {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "mock write failure",
                    ));
                }
            }
            self.written += 1;
            self.log.lock().expect("log lock").push(bytes.to_vec());
            Ok(())
        }
    }

    fn online_shared(has_data: bool) -> Arc<RwLock<data::SharedState>> {
        let mut state = data::SharedState::default();
        state.conn = data::ConnStatus::Online;
        state.has_data = has_data;
        state.snap = Arc::new(fixture_source());
        Arc::new(RwLock::new(state))
    }

    fn kind_of(bytes: &[u8]) -> u8 {
        bytes[8]
    }

    fn seq_of(bytes: &[u8]) -> u64 {
        u64::from_le_bytes(bytes[10..18].try_into().expect("seq"))
    }

    #[test]
    fn push_loop_emits_hello_snapshots_bye_in_order() {
        let shared = online_shared(true);
        let stop = Arc::new(AtomicBool::new(false));
        let log: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let mut open = {
            let log = log.clone();
            move || {
                Some(Box::new(MockSink {
                    log: log.clone(),
                    fail_after: None,
                    written: 0,
                }) as Box<dyn PipeSink>)
            }
        };
        let mut now = || 1_700_000_123_000i64;
        let ticks = Arc::new(Mutex::new(0u32));
        let mut sleep = {
            let stop = stop.clone();
            let ticks = ticks.clone();
            move |_ms: u64| {
                let mut t = ticks.lock().expect("ticks");
                *t += 1;
                if *t >= 3 {
                    stop.store(true, Ordering::SeqCst);
                }
            }
        };

        let stats = run_push_loop(&shared, 10, &stop, &mut open, &mut now, &mut sleep);
        assert_eq!(stats.connects, 1);
        assert_eq!(stats.hellos, 1);
        assert_eq!(stats.snapshots, 3);
        assert_eq!(stats.byes, 1);
        assert_eq!(stats.write_errors, 0);

        let writes = log.lock().expect("log").clone();
        let kinds: Vec<u8> = writes.iter().map(|b| kind_of(b)).collect();
        assert_eq!(kinds, vec![1, 3, 3, 3, 6]);
        assert_eq!(seq_of(&writes[1]), 1);
        assert_eq!(seq_of(&writes[2]), 2);
        assert_eq!(seq_of(&writes[3]), 3);
        assert_eq!(seq_of(&writes[4]), 4);
        // 快照 payload 必须与映射结果一致（ts_ms 取本次采样时刻）。
        // `pc` 是本机实时采样（随机器变化），比对时剔除。
        let expected: serde_json::Value = serde_json::from_str(
            &map_snapshot(&fixture_source(), true, 1, 1_700_000_123_000)
                .payload_json()
                .expect("json"),
        )
        .expect("expected json");
        let mut actual: serde_json::Value =
            serde_json::from_slice(&writes[1][HEADER_LEN..]).expect("actual json");
        actual.as_object_mut().expect("obj").remove("pc");
        assert_eq!(actual, expected);
    }

    #[test]
    fn push_loop_skips_snapshots_when_offline_or_no_data() {
        for has_data in [false, true] {
            let shared = online_shared(has_data);
            if !has_data {
                let mut guard = shared.write().expect("write");
                guard.has_data = false;
            }
            let stop = Arc::new(AtomicBool::new(false));
            let log: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
            let mut open = {
                let log = log.clone();
                move || {
                    Some(Box::new(MockSink {
                        log: log.clone(),
                        fail_after: None,
                        written: 0,
                    }) as Box<dyn PipeSink>)
                }
            };
            let mut now = || 0i64;
            let ticks = Arc::new(Mutex::new(0u32));
            let mut sleep = {
                let stop = stop.clone();
                let ticks = ticks.clone();
                move |_ms: u64| {
                    let mut t = ticks.lock().expect("ticks");
                    *t += 1;
                    if *t >= 2 {
                        stop.store(true, Ordering::SeqCst);
                    }
                }
            };
            let stats = run_push_loop(&shared, 10, &stop, &mut open, &mut now, &mut sleep);
            if has_data {
                assert_eq!(stats.snapshots, 2, "有数据时应推送");
            } else {
                assert_eq!(stats.snapshots, 0, "无数据时不得推送快照");
            }
            assert_eq!(stats.hellos, 1);
            assert_eq!(stats.byes, 1);
        }
    }

    #[test]
    fn push_loop_retries_once_per_cycle_when_pipe_missing() {
        let shared = online_shared(true);
        let attempts = Arc::new(Mutex::new(0u32));
        let mut open = {
            let attempts = attempts.clone();
            move || -> Option<Box<dyn PipeSink>> {
                *attempts.lock().expect("attempts") += 1;
                None
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let ticks = Arc::new(Mutex::new(0u32));
        let mut sleep = {
            let stop = stop.clone();
            let ticks = ticks.clone();
            move |_ms: u64| {
                let mut t = ticks.lock().expect("ticks");
                *t += 1;
                if *t >= 4 {
                    stop.store(true, Ordering::SeqCst);
                }
            }
        };
        let mut now = || 0i64;
        let stats = run_push_loop(&shared, 10, &stop, &mut open, &mut now, &mut sleep);
        assert_eq!(stats.connects, 0);
        assert_eq!(stats.snapshots, 0);
        assert_eq!(stats.byes, 0);
        assert_eq!(*attempts.lock().expect("attempts"), 4, "每周期仅一次尝试");
    }

    #[test]
    fn push_loop_drops_connection_on_write_error_and_reconnects() {
        let shared = online_shared(true);
        let stop = Arc::new(AtomicBool::new(false));
        let log: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let opens = Arc::new(Mutex::new(0u32));
        let mut open = {
            let log = log.clone();
            let opens = opens.clone();
            move || -> Option<Box<dyn PipeSink>> {
                let mut n = opens.lock().expect("opens");
                *n += 1;
                let first = *n == 1;
                Some(Box::new(MockSink {
                    log: log.clone(),
                    fail_after: if first { Some(0) } else { None },
                    written: 0,
                }) as Box<dyn PipeSink>)
            }
        };
        let ticks = Arc::new(Mutex::new(0u32));
        let mut sleep = {
            let stop = stop.clone();
            let ticks = ticks.clone();
            move |_ms: u64| {
                let mut t = ticks.lock().expect("ticks");
                *t += 1;
                if *t >= 4 {
                    stop.store(true, Ordering::SeqCst);
                }
            }
        };
        let mut now = || 0i64;
        let stats = run_push_loop(&shared, 10, &stop, &mut open, &mut now, &mut sleep);
        assert_eq!(stats.write_errors, 1);
        assert_eq!(stats.hellos, 1, "失败的那次 Hello 不计入成功");
        assert_eq!(stats.connects, 1);
        assert!(stats.snapshots >= 1, "重连后应恢复推送");
        assert_eq!(stats.byes, 1);
    }
}
