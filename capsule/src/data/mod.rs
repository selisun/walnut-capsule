//! 数据源抽象层。
//! DataSource trait：当前实现 WsSource（连 NAS）与 FakeSource（测试用）。

pub mod fake;
pub mod ws;

pub use fake::FakeSource;
pub use ws::WsSource;

use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;

/// 数据更新时向 UI 线程投递的消息（wndproc 触发重绘）。
pub const WM_APP_DATA: u32 = 0x8000 + 1; // WM_APP + 1

/// 一次全量/增量快照。WS 增量推送缺失的分区为 None，不覆盖旧值。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub cpu: Option<Cpu>,
    #[serde(default)]
    pub memory: Option<Memory>,
    #[serde(default)]
    pub disks: Option<Vec<Disk>>,
    #[serde(default)]
    pub gpus: Option<Vec<Gpu>>,
    #[serde(default)]
    pub network: Option<Vec<NetIf>>,
}

impl Snapshot {
    /// 将增量推送合并进已有快照（None 分区保留旧值）。
    pub fn merge(&mut self, inc: &Snapshot) {
        if inc.timestamp != 0 {
            self.timestamp = inc.timestamp;
        }
        if inc.cpu.is_some() {
            self.cpu = inc.cpu.clone();
        }
        if inc.memory.is_some() {
            self.memory = inc.memory.clone();
        }
        if inc.disks.is_some() {
            self.disks = inc.disks.clone();
        }
        if inc.gpus.is_some() {
            self.gpus = inc.gpus.clone();
        }
        if inc.network.is_some() {
            self.network = inc.network.clone();
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Cpu {
    #[serde(default)]
    pub usage: Option<f64>,
    #[serde(default)]
    pub temp: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Memory {
    #[serde(default)]
    pub usage: Option<f64>,
    #[serde(default)]
    pub used: Option<u64>,
    #[serde(default)]
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Disk {
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub temp: Option<i64>,
    #[serde(default)]
    pub read_bytes_per_sec: Option<u64>,
    #[serde(default)]
    pub write_bytes_per_sec: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Gpu {
    #[serde(default)]
    pub index: i32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub temp: Option<i64>,
    #[serde(default)]
    pub utilization: Option<i32>,
    #[serde(default)]
    pub memory_used: Option<u64>,
    #[serde(default)]
    pub memory_total: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetIf {
    #[serde(default, rename = "interface")]
    pub name: String,
    #[serde(default)]
    pub rx_bytes_per_sec: Option<u64>,
    #[serde(default)]
    pub tx_bytes_per_sec: Option<u64>,
}

/// 连接状态，供 UI 显示。
#[derive(Debug, Clone)]
pub enum ConnStatus {
    Connecting,
    Online,
    /// 断线，n 秒后重连
    Reconnect(u64),
}

/// 订阅详细程度：收起只订 5 项，展开订全量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Collapsed,
    Expanded,
}

/// UI → 数据源 命令。
pub enum SourceCmd {
    SetDetail(Detail),
    /// 更换 NAS 地址（"host:port"），立即重连。
    SetTarget(String),
    Shutdown,
}

/// UI 与数据源共享的状态。
pub struct SharedState {
    pub conn: ConnStatus,
    pub snap: Snapshot,
    pub has_data: bool,
    seq: AtomicU64,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            conn: ConnStatus::Connecting,
            snap: Snapshot::default(),
            has_data: false,
            seq: AtomicU64::new(0),
        }
    }
}

impl SharedState {
    pub fn seq(&self) -> u64 {
        self.seq.load(Ordering::Relaxed)
    }
    pub fn bump_seq(&self) {
        self.seq.fetch_add(1, Ordering::Relaxed);
    }
}

/// 数据源启动上下文。
pub struct SourceCtx {
    /// 主窗口句柄（用于 PostMessage 通知重绘）。
    pub hwnd: isize,
    pub state: Arc<RwLock<SharedState>>,
    pub cmd_rx: Receiver<SourceCmd>,
    /// 初始 NAS 地址 "host:port"。
    pub target: String,
}

/// 数据源接口。实现者阻塞运行在自己的线程里，直到 Shutdown。
pub trait DataSource: Send + 'static {
    fn run(self, cx: SourceCtx);
    fn spawn(self, cx: SourceCtx) -> JoinHandle<()>
    where
        Self: Sized,
    {
        std::thread::spawn(move || self.run(cx))
    }
}

/// 通知 UI 有新数据。
pub fn post_update(hwnd: isize) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    unsafe {
        let _ = PostMessageW(
            HWND(hwnd as *mut std::ffi::c_void),
            WM_APP_DATA,
            WPARAM(0),
            LPARAM(0),
        );
    }
}
