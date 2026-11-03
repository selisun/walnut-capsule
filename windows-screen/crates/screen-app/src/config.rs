//! `screen/config.toml` 模型与校验。**不含任何 NAS 地址/Token 字段**。

use std::fmt;
use std::path::Path;

use screen_core::ComponentKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Io(String),
    Parse(String),
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(msg) => write!(f, "配置读取失败：{msg}"),
            ConfigError::Parse(msg) => write!(f, "配置解析失败：{msg}"),
            ConfigError::Invalid(msg) => write!(f, "配置非法：{msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceConfig {
    pub selected: String,
    pub auto_select: bool,
    pub exclusive: bool,
    pub max_devices: u8,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            selected: String::new(),
            auto_select: true,
            exclusive: true,
            max_devices: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub theme: String,
    pub layout_template: String,
    pub orientation: String,
    pub flip: String,
    pub brightness: u8,
    pub refresh_secs: u64,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            theme: "default".to_string(),
            layout_template: "auto".to_string(),
            orientation: "auto".to_string(),
            flip: "normal".to_string(),
            brightness: 100,
            refresh_secs: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ComponentVisibility {
    pub visible: Vec<ComponentKind>,
    pub order: Vec<ComponentKind>,
}

impl Default for ComponentVisibility {
    fn default() -> Self {
        let visible = screen_core::Components::default().visible;
        Self {
            order: visible.clone(),
            visible,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeSettings {
    pub font_scale: f32,
    pub background: String,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            font_scale: 1.0,
            background: String::new(),
        }
    }
}

/// IPC 对端校验的默认客户端映像文件名（生产主客户端）。
/// T5 真机回归期间可在配置中覆盖为推送模拟器文件名（如 `snapshot-pusher.exe`）。
pub const DEFAULT_CLIENT_EXE: &str = "hetao-capsule.exe";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IntegrationConfig {
    /// 已批准：仅 IPC，小屏不连 NAS。
    pub mode: String,
    pub pipe: String,
    pub stale_ms: i64,
    /// IPC 对端校验期望的客户端映像文件名（仅文件名，不含路径）。
    /// 生产默认 `hetao-capsule.exe`；T5 真机回归时可改为推送模拟器的文件名。
    pub client_exe: String,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            mode: "ipc".to_string(),
            pipe: String::new(),
            stale_ms: 3_000,
            client_exe: DEFAULT_CLIENT_EXE.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub device: DeviceConfig,
    pub display: DisplayConfig,
    pub components: ComponentVisibility,
    pub theme: ThemeSettings,
    pub integration: IntegrationConfig,
    pub logging: LoggingConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            device: DeviceConfig::default(),
            display: DisplayConfig::default(),
            components: ComponentVisibility::default(),
            theme: ThemeSettings::default(),
            integration: IntegrationConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

impl Config {
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let cfg: Config = toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io(format!("{}: {e}", path.display())))?;
        Self::from_toml_str(&text)
    }

    /// 缺失文件时返回默认配置，并标记来源。
    pub fn load_or_default(path: &Path) -> Result<(Self, bool), ConfigError> {
        if !path.exists() {
            return Ok((Self::default(), true));
        }
        Ok((Self::load(path)?, false))
    }

    pub fn to_toml_string(&self) -> Result<String, ConfigError> {
        toml::to_string(self).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// 持久化配置；父目录不存在时创建。写入前调用方应先 validate。
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = self.to_toml_string()?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| ConfigError::Io(format!("{}: {e}", parent.display())))?;
            }
        }
        std::fs::write(path, text).map_err(|e| ConfigError::Io(format!("{}: {e}", path.display())))
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != 1 {
            return Err(ConfigError::Invalid(format!(
                "version 必须为 1（实际 {}）",
                self.version
            )));
        }
        if self.device.max_devices != 1 {
            return Err(ConfigError::Invalid(
                "device.max_devices 首期必须为 1".to_string(),
            ));
        }
        if !(1..=10).contains(&self.display.refresh_secs) {
            return Err(ConfigError::Invalid(
                "display.refresh_secs 范围 1-10".to_string(),
            ));
        }
        if self.display.brightness > 100 {
            return Err(ConfigError::Invalid(
                "display.brightness 范围 0-100".to_string(),
            ));
        }
        if !self.display.orientation.is_empty()
            && !["auto", "portrait", "landscape"].contains(&self.display.orientation.as_str())
        {
            return Err(ConfigError::Invalid("display.orientation".to_string()));
        }
        if !["normal", "flipped"].contains(&self.display.flip.as_str()) {
            return Err(ConfigError::Invalid("display.flip".to_string()));
        }
        if !(0.5..=3.0).contains(&self.theme.font_scale) {
            return Err(ConfigError::Invalid(
                "theme.font_scale 范围 0.5-3.0".to_string(),
            ));
        }
        if self.integration.mode != "ipc" {
            return Err(ConfigError::Invalid(
                "integration.mode 只允许 ipc（小屏不得连线 NAS）".to_string(),
            ));
        }
        if self.integration.stale_ms < 1_000 {
            return Err(ConfigError::Invalid(
                "integration.stale_ms 不得小于 1000".to_string(),
            ));
        }
        validate_client_exe(&self.integration.client_exe)?;
        if self.components.visible.is_empty() {
            return Err(ConfigError::Invalid(
                "components.visible 不能为空".to_string(),
            ));
        }
        if !["info", "warn", "error"].contains(&self.logging.level.as_str()) {
            return Err(ConfigError::Invalid("logging.level".to_string()));
        }
        Ok(())
    }
}

/// 对端映像名校验：非空、仅文件名（不含路径/盘符）、以 `.exe` 结尾（大小写不敏感）。
fn validate_client_exe(name: &str) -> Result<(), ConfigError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::Invalid(
            "integration.client_exe 不能为空".to_string(),
        ));
    }
    if trimmed != name {
        return Err(ConfigError::Invalid(
            "integration.client_exe 不得包含首尾空白".to_string(),
        ));
    }
    if name.contains(['\\', '/', ':']) {
        return Err(ConfigError::Invalid(
            "integration.client_exe 只允许文件名，不得含路径".to_string(),
        ));
    }
    if !name.to_ascii_lowercase().ends_with(".exe") {
        return Err(ConfigError::Invalid(
            "integration.client_exe 必须以 .exe 结尾".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let cfg = Config::default();
        cfg.validate().expect("default config");
        assert_eq!(cfg.integration.mode, "ipc");
        assert_eq!(cfg.device.max_devices, 1);
        assert_eq!(cfg.display.flip, "normal");
    }

    #[test]
    fn old_config_without_flip_defaults_to_normal() {
        let text = r#"
version = 1
[display]
orientation = "portrait"
"#;
        let cfg = Config::from_toml_str(text).expect("config");
        assert_eq!(cfg.display.flip, "normal");
    }

    #[test]
    fn invalid_flip_is_rejected() {
        let text = r#"
version = 1
[display]
flip = "mirror"
"#;
        assert!(Config::from_toml_str(text).is_err());
    }

    #[test]
    fn config_save_and_reload_preserves_direction() {
        let path = std::env::temp_dir().join(format!(
            "hetao-screen-config-roundtrip-{}.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut cfg = Config::default();
        cfg.display.orientation = "landscape".to_string();
        cfg.display.flip = "flipped".to_string();
        cfg.save(&path).expect("save");
        let loaded = Config::load(&path).expect("load");
        assert_eq!(loaded.display.orientation, "landscape");
        assert_eq!(loaded.display.flip, "flipped");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let text = r#"
version = 1
unknown_key = "ignored"
[device]
max_devices = 1
"#;
        let cfg = Config::from_toml_str(text).expect("config");
        assert_eq!(cfg.version, 1);
    }

    #[test]
    fn multi_device_is_rejected() {
        let text = r#"
version = 1
[device]
max_devices = 2
"#;
        assert!(Config::from_toml_str(text).is_err());
    }

    #[test]
    fn standalone_nas_mode_is_rejected() {
        let text = r#"
version = 1
[integration]
mode = "standalone"
"#;
        assert!(Config::from_toml_str(text).is_err());
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        let text = r#"
version = 1
[display]
refresh_secs = 0
brightness = 120
[theme]
font_scale = 9.0
"#;
        assert!(Config::from_toml_str(text).is_err());
    }

    #[test]
    fn missing_file_yields_defaults() {
        let path = std::env::temp_dir().join("hetao-screen-missing-config.toml");
        let _ = std::fs::remove_file(&path);
        let (cfg, used_default) = Config::load_or_default(&path).expect("load");
        assert!(used_default);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn default_client_exe_is_capsule() {
        let cfg = Config::default();
        assert_eq!(cfg.integration.client_exe, "hetao-capsule.exe");
        cfg.validate().expect("default valid");
    }

    #[test]
    fn old_config_without_client_exe_uses_default() {
        let text = r#"
version = 1
[integration]
mode = "ipc"
pipe = "hetao-screen"
"#;
        let cfg = Config::from_toml_str(text).expect("config");
        assert_eq!(cfg.integration.client_exe, "hetao-capsule.exe");
    }

    #[test]
    fn client_exe_can_be_overridden_for_t5_simulator() {
        let text = r#"
version = 1
[integration]
client_exe = "snapshot-pusher.exe"
"#;
        let cfg = Config::from_toml_str(text).expect("config");
        assert_eq!(cfg.integration.client_exe, "snapshot-pusher.exe");
    }

    #[test]
    fn client_exe_with_path_or_non_exe_is_rejected() {
        for bad in [
            r"C:\tools\pusher.exe",
            "tools/pusher.exe",
            "pusher",
            "pusher.bat",
            "",
            "  ",
        ] {
            let text = format!("version = 1\n[integration]\nclient_exe = \"{bad}\"\n");
            assert!(
                Config::from_toml_str(&text).is_err(),
                "应拒绝 client_exe={bad:?}"
            );
        }
    }

    #[test]
    fn client_exe_case_insensitive_exe_suffix_is_accepted() {
        let text = r#"
version = 1
[integration]
client_exe = "Snapshot-Pusher.EXE"
"#;
        let cfg = Config::from_toml_str(text).expect("config");
        assert_eq!(cfg.integration.client_exe, "Snapshot-Pusher.EXE");
    }
}
