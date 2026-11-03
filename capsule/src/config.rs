//! 配置读写：%APPDATA%\hetao-capsule\config.toml
//! 结构按 nas / capsule 分区，其余分区预留。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub nas: NasConfig,
    #[serde(default)]
    pub capsule: CapsuleConfig,
    #[serde(default)]
    pub screen: ScreenConfig,
}

/// 外接小屏（图灵智显）设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenConfig {
    /// 显示模式：capsule（悬浮胶囊）| screen（机箱小屏）
    #[serde(default = "default_screen_mode")]
    pub mode: String,
    /// 显示方向：portrait | portrait-flip | landscape | landscape-flip
    #[serde(default = "default_screen_orientation")]
    pub orientation: String,
    /// 小屏配色主题：dark | bignum-light | light | bignum-dark（顺序同 SCREEN_THEMES）
    #[serde(default = "default_screen_theme")]
    pub theme: String,
    /// 小屏刷新间隔（秒）：1 | 2 | 3 | 5
    #[serde(default = "default_screen_interval")]
    pub interval: u32,
    /// 小屏开关：关闭后不再拉起 screen.ps1。
    /// 默认关闭；旧配置里已有该字段就沿用，升级不会改动用户原来的开关状态。
    #[serde(default)]
    pub enabled: bool,
    /// 小屏引擎：powershell（默认，托管旧脚本）| native（Rust 小屏进程 + 快照推送）
    #[serde(default = "default_screen_engine")]
    pub engine: String,
    /// 小屏显示项目（与胶囊面板的卡片开关相互独立）
    #[serde(default = "default_true")]
    pub cpu: bool,
    #[serde(default = "default_true")]
    pub memory: bool,
    #[serde(default = "default_true")]
    pub gpu: bool,
    #[serde(default = "default_true")]
    pub network: bool,
    #[serde(default = "default_true")]
    pub disk: bool,
}

impl Default for ScreenConfig {
    fn default() -> Self {
        Self {
            mode: default_screen_mode(),
            orientation: default_screen_orientation(),
            theme: default_screen_theme(),
            interval: default_screen_interval(),
            enabled: false,
            engine: default_screen_engine(),
            cpu: true,
            memory: true,
            gpu: true,
            network: true,
            disk: true,
        }
    }
}

/// 显示模式档位（与设置窗单选顺序一致）：胶囊模式 / 三环模式 / 机箱小屏模式。
pub const SCREEN_MODE_CAPSULE: &str = "capsule";
pub const SCREEN_MODE_RINGS: &str = "rings";
pub const SCREEN_MODE_SCREEN: &str = "screen";
pub const SCREEN_MODES: [&str; 3] = [SCREEN_MODE_CAPSULE, SCREEN_MODE_RINGS, SCREEN_MODE_SCREEN];

/// 小屏方向档位（与设置窗单选顺序一致）。
pub const SCREEN_ORIENTATIONS: [&str; 4] =
    ["portrait", "portrait-flip", "landscape", "landscape-flip"];

/// 小屏配色主题档位（与设置窗下拉框顺序一致）：Dark / 白色大字 / Light / 黑色大字。
pub const SCREEN_THEMES: [&str; 4] = ["dark", "bignum-light", "light", "bignum-dark"];

/// 小屏刷新间隔档位（秒，与设置窗下拉框顺序一致）。
pub const SCREEN_INTERVALS: [u32; 4] = [1, 2, 3, 5];

/// 小屏引擎档位（B-10）。默认 powershell，升级后行为不变。
pub const SCREEN_ENGINE_POWERSHELL: &str = "powershell";
pub const SCREEN_ENGINE_NATIVE: &str = "native";
pub const SCREEN_ENGINES: [&str; 2] = [SCREEN_ENGINE_POWERSHELL, SCREEN_ENGINE_NATIVE];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NasConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// 配对令牌：与服务端 CAPSULE_TOKEN / 启动日志中的令牌一致
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsuleConfig {
    /// 窗口左上角；None 表示自动（右上角）。
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    /// 胶囊整体不透明度 0.30..1.00
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    /// 主题：light（亮色）| dark（暗色）
    #[serde(default = "default_theme")]
    pub theme: String,
    /// 固定在任务栏（显示在任务栏上，位置锁定、始终浮于任务栏之上）
    #[serde(default)]
    pub taskbar_pin: bool,
    /// 固定在任务栏时用户拖动选择的左边缘 x（屏幕坐标）；None 表示自动选取空位
    #[serde(default)]
    pub taskbar_x: Option<i32>,
    /// 显示方向：auto（拖动到左右边缘自动竖排）| h（横向药丸）| v（竖向细长条）
    #[serde(default = "default_orientation")]
    pub orientation: String,
    /// 展开样式：panel（320 小面板）| big（贴屏幕侧边滑出的超大卡片）
    #[serde(default = "default_expand_style")]
    pub expand_style: String,
    /// 展开面板各类卡片的显示开关（收起态与任务栏三环不受影响）
    #[serde(default)]
    pub cards: CardVis,
    /// 点击面板外任意位置自动收起展开面板（默认开启）
    #[serde(default = "default_true")]
    pub click_outside_collapse: bool,
    /// 全屏应用（游戏、放映等）运行时自动隐藏悬浮窗（默认开启）
    #[serde(default = "default_true")]
    pub hide_on_fullscreen: bool,
}

impl Default for CapsuleConfig {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            opacity: 1.0,
            theme: default_theme(),
            taskbar_pin: false,
            taskbar_x: None,
            orientation: default_orientation(),
            expand_style: default_expand_style(),
            cards: CardVis::default(),
            click_outside_collapse: true,
            hide_on_fullscreen: true,
        }
    }
}

/// 主题档位（与设置窗单选顺序一致）。
pub const THEMES: [&str; 2] = ["light", "dark"];

impl CapsuleConfig {
    /// 是否暗色主题（除 "dark" 外都按亮色处理）。
    pub fn theme_dark(&self) -> bool {
        self.theme == THEMES[1]
    }
}

/// 显示方向档位（与设置窗单选顺序一致）。
pub const ORIENTATIONS: [&str; 3] = ["auto", "h", "v"];

/// 展开样式档位（与设置窗单选顺序一致）：小面板 / 超大卡片。
pub const EXPAND_STYLES: [&str; 2] = ["panel", "big"];

impl CapsuleConfig {
    /// 是否使用超大卡片作为展开形态（"big" 之外都按小面板处理）。
    pub fn expand_big(&self) -> bool {
        self.expand_style == EXPAND_STYLES[1]
    }
}

impl CapsuleConfig {
    /// 方向偏好：Some(true)=竖排，Some(false)=横排，None=自动。
    pub fn orientation_hint(&self) -> Option<bool> {
        match self.orientation.as_str() {
            "v" => Some(true),
            "h" => Some(false),
            _ => None,
        }
    }
}

/// 展开面板的卡片数量：CPU / 内存 / 显卡 / 网络 / 硬盘读写 / 存储。
/// 下标与 `CardVis::all()`、设置窗复选框顺序一致。
pub const CARD_COUNT: usize = 6;

/// 卡片默认显示顺序（下标即默认次序）。
pub const DEFAULT_CARD_ORDER: [u8; CARD_COUNT] = [0, 1, 2, 3, 4, 5];

fn default_card_order() -> [u8; CARD_COUNT] {
    DEFAULT_CARD_ORDER
}

/// 展开面板的卡片显示开关。缺字段一律视为显示，旧配置零迁移。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardVis {
    #[serde(default = "default_true")]
    pub cpu: bool,
    #[serde(default = "default_true")]
    pub memory: bool,
    #[serde(default = "default_true")]
    pub gpu: bool,
    #[serde(default = "default_true")]
    pub network: bool,
    /// 硬盘读写心电图
    #[serde(default = "default_true")]
    pub disk_io: bool,
    /// 存储状态（卷用量 / 盘用量）
    #[serde(default = "default_true")]
    pub storage: bool,
    /// 显示顺序：`CARD_COUNT` 个卡片下标的一个排列，记录用户在设置里的勾选先后
    /// （最近勾选的排最后）。缺字段时用默认顺序。
    #[serde(default = "default_card_order")]
    pub order: [u8; CARD_COUNT],
}

impl Default for CardVis {
    fn default() -> Self {
        Self {
            cpu: true,
            memory: true,
            gpu: true,
            network: true,
            disk_io: true,
            storage: true,
            order: DEFAULT_CARD_ORDER,
        }
    }
}

impl CardVis {
    /// 是否至少留下一张卡片（设置界面禁止全关）。
    pub fn any(&self) -> bool {
        self.cpu || self.memory || self.gpu || self.network || self.disk_io || self.storage
    }
    /// 按设置界面的顺序返回各项当前值，供复选框初始化与"至少保留一项"判定。
    pub fn all(&self) -> [bool; 6] {
        [
            self.cpu,
            self.memory,
            self.gpu,
            self.network,
            self.disk_io,
            self.storage,
        ]
    }
    /// 按设置界面的顺序写入各项。
    pub fn set_all(&mut self, v: [bool; 6]) {
        self.cpu = v[0];
        self.memory = v[1];
        self.gpu = v[2];
        self.network = v[3];
        self.disk_io = v[4];
        self.storage = v[5];
    }
    /// 按用户勾选先后返回**已勾选**卡片的绘制顺序（其余未勾选的跳过）。
    /// `order` 里出现重复或越界下标时自动跳过，缺项按默认顺序补在后面。
    pub fn ordered(&self) -> Vec<usize> {
        let on = self.all();
        let mut seen = [false; CARD_COUNT];
        let mut out = Vec::with_capacity(CARD_COUNT);
        for &raw in self.order.iter() {
            let i = raw as usize;
            if i < CARD_COUNT && on[i] && !seen[i] {
                seen[i] = true;
                out.push(i);
            }
        }
        for i in 0..CARD_COUNT {
            if on[i] && !seen[i] {
                out.push(i);
            }
        }
        out
    }
}

/// 把第 `idx` 张卡片挪到顺序末尾并补全成完整排列（记录勾选先后用）。
pub fn touch_order(order: &mut [u8; CARD_COUNT], idx: usize) {
    let mut v: Vec<u8> = order
        .iter()
        .copied()
        .filter(|&x| x as usize != idx)
        .collect();
    v.push(idx as u8);
    // 补全缺失下标，保证仍是 0..CARD_COUNT 的完整排列
    for i in 0..CARD_COUNT {
        if !v.contains(&(i as u8)) {
            v.push(i as u8);
        }
    }
    order.copy_from_slice(&v[..CARD_COUNT]);
}

impl Default for NasConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            token: String::new(),
        }
    }
}

fn default_host() -> String {
    "192.168.1.100".to_string()
}
fn default_port() -> u16 {
    8765
}
fn default_opacity() -> f32 {
    1.0
}
fn default_true() -> bool {
    true
}
fn default_orientation() -> String {
    ORIENTATIONS[0].to_string()
}
fn default_expand_style() -> String {
    EXPAND_STYLES[0].to_string()
}
fn default_theme() -> String {
    THEMES[0].to_string()
}
fn default_screen_mode() -> String {
    SCREEN_MODES[0].to_string()
}
fn default_screen_orientation() -> String {
    SCREEN_ORIENTATIONS[0].to_string()
}
fn default_screen_theme() -> String {
    SCREEN_THEMES[0].to_string()
}
fn default_screen_interval() -> u32 {
    2
}
fn default_screen_engine() -> String {
    SCREEN_ENGINE_POWERSHELL.to_string()
}

impl NasConfig {
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
    /// 是否尚未配置：令牌为空，或地址仍是占位默认值。
    /// 注意：端口 8765 是正常默认端口，不能作为“未配置”的判据（否则每次启动都弹设置窗）。
    pub fn needs_setup(&self) -> bool {
        self.token.trim().is_empty() || self.host == default_host()
    }
    /// 解析 "host" 或 "host:port"。
    pub fn set_addr(&mut self, s: &str) -> bool {
        let s = s.trim().trim_start_matches("ws://").trim_matches('/');
        if s.is_empty() {
            return false;
        }
        match s.rsplit_once(':') {
            Some((h, p)) if !h.is_empty() => match p.parse::<u16>() {
                Ok(port) => {
                    self.host = h.to_string();
                    self.port = port;
                    true
                }
                Err(_) => {
                    // "host" 中无端口（IPv4/域名不含冒号时 rsplit 不会误判）
                    if s.parse::<u16>().is_err() && !s.contains(':') {
                        self.host = s.to_string();
                        true
                    } else {
                        false
                    }
                }
            },
            _ => {
                self.host = s.to_string();
                true
            }
        }
    }
}

pub fn config_path() -> PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hetao-capsule");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("config.toml")
}

pub fn load() -> Config {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(mut cfg) => {
                // 旧版本的小屏主题值归一：white→light、official（浅色）→bignum-light，
                // forest、nas、tuning（黑绿）已下线，一律回 dark
                if cfg.screen.theme == "white" {
                    cfg.screen.theme = "light".to_string();
                } else if cfg.screen.theme == "official" {
                    cfg.screen.theme = "bignum-light".to_string();
                } else if cfg.screen.theme == "tuning"
                    || cfg.screen.theme == "forest"
                    || cfg.screen.theme == "nas"
                {
                    cfg.screen.theme = "dark".to_string();
                }
                // 小屏引擎归一：未知值一律回退 powershell（保持旧行为，不因配置笔误改变链路）
                if cfg.screen.engine != SCREEN_ENGINE_NATIVE {
                    cfg.screen.engine = SCREEN_ENGINE_POWERSHELL.to_string();
                }
                cfg
            }
            Err(e) => {
                // 配置损坏：记录错误并回退到默认值（避免启动失败）
                eprintln!("警告：配置文件解析失败（{}），使用默认配置", e);
                Config::default()
            }
        },
        Err(_) => {
            // 首次运行：写一份默认配置方便用户手改
            let cfg = Config::default();
            let _ = save(&cfg);
            cfg
        }
    }
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let text = toml::to_string_pretty(cfg).unwrap_or_default();
    std::fs::write(config_path(), text)
}
