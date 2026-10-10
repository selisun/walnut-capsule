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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NasConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// 配对令牌：与服务端 CAPSULE_TOKEN / 启动日志中的令牌一致
    #[serde(default)]
    pub token: String,
    /// 数据源：ws（连 NAS）| fake（模拟数据）
    #[serde(default = "default_source")]
    pub source: String,
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
    /// 磨砂背景（系统合成器 Acrylic）
    #[serde(default)]
    pub frost: bool,
    /// 固定在任务栏（显示在任务栏上，位置锁定、始终浮于任务栏之上）
    #[serde(default)]
    pub taskbar_pin: bool,
    /// 固定在任务栏时用户拖动选择的左边缘 x（屏幕坐标）；None 表示自动选取空位
    #[serde(default)]
    pub taskbar_x: Option<i32>,
}

impl Default for CapsuleConfig {
    fn default() -> Self {
        Self { x: None, y: None, opacity: 1.0, frost: false, taskbar_pin: false, taskbar_x: None }
    }
}

impl Default for NasConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            token: String::new(),
            source: default_source(),
        }
    }
}



fn default_host() -> String {
    "192.168.1.100".to_string()
}
fn default_port() -> u16 {
    8765
}
fn default_source() -> String {
    "ws".to_string()
}
fn default_opacity() -> f32 {
    1.0
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
        Ok(text) => toml::from_str(&text).unwrap_or_default(),
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
