//! 数据源抽象层。
//! DataSource trait：当前实现 WsSource（连 NAS）。

pub mod ws;

pub use ws::WsSource;

use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;

use crate::config::CardVis;

/// 数据更新时向 UI 线程投递的消息（wndproc 触发重绘）。
pub const WM_APP_DATA: u32 = 0x8000 + 1; // WM_APP + 1
/// 后台面板构建线程完成一帧时投递的消息（UI 取结果并重绘）。
pub const WM_APP_PANEL: u32 = 0x8000 + 3; // WM_APP + 3（+2 为托盘回调）

/// 一次全量/增量快照。WS 增量推送缺失的分区为 None，不覆盖旧值。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub cpu: Option<Cpu>,
    #[serde(default)]
    pub memory: Option<Memory>,
    #[serde(default)]
    pub disks: Option<Vec<Disk>>,
    /// 逻辑卷（仅用于按总量匹配归属到物理盘，不单独展示）
    #[serde(default)]
    pub volumes: Option<Vec<Volume>>,
    #[serde(default)]
    pub gpus: Option<Vec<Gpu>>,
    #[serde(default)]
    pub network: Option<Vec<NetIf>>,
    /// 目标网卡的累计流量（今日 / 本月 / 总量）
    #[serde(default)]
    pub traffic: Option<Traffic>,
}

impl Snapshot {
    /// 将增量推送合并进已有快照（None 分区保留旧值）。
    pub fn merge(&mut self, inc: &Snapshot) {
        if inc.timestamp != 0 {
            self.timestamp = inc.timestamp;
        }
        if inc.hostname.is_some() {
            self.hostname = inc.hostname.clone();
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
        if inc.volumes.is_some() {
            self.volumes = inc.volumes.clone();
        }
        if inc.gpus.is_some() {
            self.gpus = inc.gpus.clone();
        }
        if inc.network.is_some() {
            self.network = inc.network.clone();
        }
        if inc.traffic.is_some() {
            self.traffic = inc.traffic.clone();
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Cpu {
    #[serde(default)]
    pub usage: Option<f64>,
    #[serde(default)]
    pub temp: Option<f64>,
    /// 平均负载（1 / 5 / 15 分钟），老服务端没有这三个字段
    #[serde(default)]
    pub load1: Option<f64>,
    #[serde(default)]
    pub load5: Option<f64>,
    #[serde(default)]
    pub load15: Option<f64>,
    /// 30 分钟平均负载（服务端对 load1 采样自算，采样覆盖满 30 分钟才有值）
    #[serde(default)]
    pub load30: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Memory {
    #[serde(default)]
    pub usage: Option<f64>,
    #[serde(default)]
    pub used: Option<u64>,
    #[serde(default)]
    pub total: Option<u64>,
    /// 缓存（Buffers + Cached）
    #[serde(default)]
    pub cache: Option<u64>,
    /// 可用（真正的空闲内存）
    #[serde(default)]
    pub free: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Disk {
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub read_bytes_per_sec: Option<u64>,
    #[serde(default)]
    pub write_bytes_per_sec: Option<u64>,
    #[serde(default)]
    pub used_bytes: Option<u64>,
    #[serde(default)]
    pub total_bytes: Option<u64>,
}

/// 逻辑存储卷（fnOS /volN 等）：只关心容量与用量。
#[derive(Debug, Clone, Deserialize)]
pub struct Volume {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub used_bytes: Option<u64>,
    #[serde(default)]
    pub total_bytes: Option<u64>,
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
    /// 网卡协商速率（Mbps）；0/缺失表示未知，网络环退化为只画轨道
    #[serde(default)]
    pub speed_mbps: Option<u32>,
}

/// 目标网卡的累计流量（字节）：今日 / 昨日 / 最近 7 日 / 最近 30 日，均含当天，收发分开。
#[derive(Debug, Clone, Deserialize)]
pub struct Traffic {
    #[serde(default)]
    pub rx_today: u64,
    #[serde(default)]
    pub tx_today: u64,
    #[serde(default)]
    pub rx_yesterday: u64,
    #[serde(default)]
    pub tx_yesterday: u64,
    #[serde(default)]
    pub rx_days7: u64,
    #[serde(default)]
    pub tx_days7: u64,
    #[serde(default)]
    pub rx_days30: u64,
    #[serde(default)]
    pub tx_days30: u64,
    /// 最近 30 天按天序列（旧 → 新），供右栏柱状图
    #[serde(default)]
    pub days_rx: Vec<u64>,
    #[serde(default)]
    pub days_tx: Vec<u64>,
    /// 与序列一一对应的日期（"10-25"），悬停提示用
    #[serde(default)]
    pub dates: Vec<String>,
}

/// 连接状态，供 UI 显示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnStatus {
    Connecting,
    Online,
    /// 断线，n 秒后重连
    Reconnect(u64),
    /// 令牌错误 or 403，需重新设置
    AuthFailed,
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
    /// 更换 NAS 地址与配对令牌（"host:port", token），立即重连。
    SetTarget(String, String),
    Shutdown,
}

/// UI 与数据源共享的状态。
pub struct SharedState {
    pub conn: ConnStatus,
    /// 快照整体 Arc 化：WS 线程写时 clone-modify-swap（写锁临界区极短），
    /// UI 线程渲染前只 clone Arc，全程不持锁，杜绝锁竞争造成的帧尖峰。
    pub snap: Arc<Snapshot>,
    pub has_data: bool,
    /// 暗色主题：上屏前把画面里的中性色翻转（白底↔黑底、深字↔浅字），彩色保持不动。
    pub dark: bool,
    /// 当前连接的 NAS 地址（用户配置的 host，用于展开态顶栏显示）。
    pub nas_host: String,
    /// 是否固定在任务栏（透明融入任务栏，不画卡片底）。
    pub pinned: bool,
    /// 任务栏是否为浅色主题（决定固定在任务栏时的文字配色）。
    pub taskbar_light: bool,
    /// 固定在任务栏且任务栏位于屏幕下方时为 true：展开面板向上生长（顶栏贴窗口底部）。
    pub expand_up: bool,
    /// 竖排形态（细长条胶囊）；展开时面板从竖条一侧推出（不向下生长）。
    pub vertical: bool,
    /// 竖排且面板在竖条左侧（胶囊贴屏幕右缘）时为 true：窗口右缘固定、面板向左推。
    pub panel_left: bool,
    /// 展开样式为超大卡片（贴屏幕侧边滑出，尺寸由 app 计算后通过 View::set_big_geometry 下发）。
    pub big: bool,
    /// 展开面板卡片显示开关：UI 线程算窗口高度、构建线程画卡片必须共用这一份。
    pub cards: CardVis,
    seq: AtomicU64,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            conn: ConnStatus::Connecting,
            snap: Arc::new(Snapshot::default()),
            has_data: false,
            dark: false,
            nas_host: String::new(),
            pinned: false,
            taskbar_light: true,
            expand_up: false,
            vertical: false,
            panel_left: false,
            big: false,
            cards: CardVis::default(),
            seq: AtomicU64::new(0),
        }
    }
}

/// 一帧渲染所需数据的轻量视图：持锁期间只拷贝 Arc 指针与少量小字段，
/// 渲染在锁外进行。
pub struct Frame<'a> {
    /// 快照的 Arc 指针引用（供视图把构建任务派发到后台线程）
    pub arc: &'a Arc<Snapshot>,
    pub snap: &'a Snapshot,
    /// 数据版本号：仅在快照/连接状态变化时递增，视图据此复用整帧缓存。
    pub seq: u64,
    pub has_data: bool,
    /// 是否暗色主题。
    pub dark: bool,
    pub conn: &'a ConnStatus,
    /// 当前连接的 NAS 地址（host）。
    pub nas_host: &'a str,
    /// 是否固定在任务栏（透明融合）。
    pub pinned: bool,
    /// 任务栏是否浅色主题。
    pub taskbar_light: bool,
    /// 展开面板是否向上生长（顶栏位于窗口底部）。
    pub expand_up: bool,
    /// 竖排形态（细长条胶囊）。
    pub vertical: bool,
    /// 竖排且面板在竖条左侧。
    pub panel_left: bool,
    /// 展开样式为超大卡片。
    pub big: bool,
    /// 展开面板卡片显示开关。
    pub cards: CardVis,
}

/// 锁外持有的一帧数据：Arc 快照指针 + 少量小字段拷贝。
pub struct OwnedFrame {
    pub snap: Arc<Snapshot>,
    pub seq: u64,
    pub conn: ConnStatus,
    pub has_data: bool,
    pub dark: bool,
    pub nas_host: String,
    pub pinned: bool,
    pub taskbar_light: bool,
    pub expand_up: bool,
    pub vertical: bool,
    pub panel_left: bool,
    pub big: bool,
    pub cards: CardVis,
}

impl OwnedFrame {
    pub fn as_frame(&self) -> Frame<'_> {
        Frame {
            arc: &self.snap,
            snap: &self.snap,
            seq: self.seq,
            has_data: self.has_data,
            dark: self.dark,
            conn: &self.conn,
            nas_host: &self.nas_host,
            pinned: self.pinned,
            taskbar_light: self.taskbar_light,
            expand_up: self.expand_up,
            vertical: self.vertical,
            panel_left: self.panel_left,
            big: self.big,
            cards: self.cards,
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
    /// 极短临界区：只 clone Arc 指针与小字段，立刻放锁，渲染在锁外。
    pub fn grab(&self) -> OwnedFrame {
        OwnedFrame {
            snap: self.snap.clone(),
            seq: self.seq(),
            conn: self.conn.clone(),
            has_data: self.has_data,
            dark: self.dark,
            nas_host: self.nas_host.clone(),
            pinned: self.pinned,
            taskbar_light: self.taskbar_light,
            expand_up: self.expand_up,
            vertical: self.vertical,
            panel_left: self.panel_left,
            big: self.big,
            cards: self.cards,
        }
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
    /// 初始配对令牌。
    pub token: String,
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
