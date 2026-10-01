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
    /// 数据源：ws（连 NAS）| fake（模拟数据）
    #[serde(default = "default_source")]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapsuleConfig {
    /// 窗口左上角；None 表示自动（右上角）。
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
}

impl Default for NasConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
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

impl NasConfig {
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
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
