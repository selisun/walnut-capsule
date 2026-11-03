//! 主题数据模型（纯数据，不含校验逻辑；校验在 `screen-theme`）。

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 8 位 RGBA 颜色。JSON 使用 `#RRGGBB` 或 `#RRGGBBAA`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 255)
    }

    pub fn from_hex(text: &str) -> Result<Self, &'static str> {
        let s = text.strip_prefix('#').ok_or("颜色必须以 # 开头")?;
        let (r, g, b, a) = match s.len() {
            6 => (
                u8::from_str_radix(&s[0..2], 16).map_err(|_| "颜色格式错误")?,
                u8::from_str_radix(&s[2..4], 16).map_err(|_| "颜色格式错误")?,
                u8::from_str_radix(&s[4..6], 16).map_err(|_| "颜色格式错误")?,
                255,
            ),
            8 => (
                u8::from_str_radix(&s[0..2], 16).map_err(|_| "颜色格式错误")?,
                u8::from_str_radix(&s[2..4], 16).map_err(|_| "颜色格式错误")?,
                u8::from_str_radix(&s[4..6], 16).map_err(|_| "颜色格式错误")?,
                u8::from_str_radix(&s[6..8], 16).map_err(|_| "颜色格式错误")?,
            ),
            _ => return Err("颜色长度必须为 6 或 8 位十六进制"),
        };
        Ok(Self::new(r, g, b, a))
    }

    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }

    /// 0..=1 的 alpha。
    pub fn alpha_f32(self) -> f32 {
        self.a as f32 / 255.0
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl FromStr for Color {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Color::from_hex(s)
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Color::from_hex(&text).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    pub bg: Color,
    pub card: Color,
    pub text: Color,
    pub dim: Color,
    pub accent: Color,
    pub warn: Color,
    pub danger: Color,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Typography {
    pub family: String,
    pub base_px: f32,
    pub scale: f32,
    #[serde(default)]
    pub weights: BTreeMap<String, u16>,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            family: "Microsoft YaHei".to_string(),
            base_px: 14.0,
            scale: 1.0,
            weights: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    #[default]
    Cover,
    Contain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageAnchor {
    #[default]
    Center,
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Background {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub fit: ImageFit,
    #[serde(default)]
    pub anchor: ImageAnchor,
    #[serde(default)]
    pub dim: f32,
    #[serde(default = "default_bg_color")]
    pub fallback_color: Color,
}

fn default_bg_color() -> Color {
    Color::rgb(16, 20, 24)
}

impl Default for Background {
    fn default() -> Self {
        Self {
            image: None,
            fit: ImageFit::Cover,
            anchor: ImageAnchor::Center,
            dim: 0.0,
            fallback_color: default_bg_color(),
        }
    }
}

/// 可显示的组件种类（与 `04` 的组件类型一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ComponentKind {
    Cpu,
    Memory,
    Gpu,
    Network,
    Disk,
    Storage,
    Temp,
    Clock,
}

impl ComponentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ComponentKind::Cpu => "cpu",
            ComponentKind::Memory => "memory",
            ComponentKind::Gpu => "gpu",
            ComponentKind::Network => "network",
            ComponentKind::Disk => "disk",
            ComponentKind::Storage => "storage",
            ComponentKind::Temp => "temp",
            ComponentKind::Clock => "clock",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ComponentStyle {
    #[serde(default)]
    pub label: Option<String>,
    /// 调色板键名（如 `accent`），不是硬编码颜色。
    #[serde(default)]
    pub accent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Components {
    #[serde(default = "default_visible")]
    pub visible: Vec<ComponentKind>,
    #[serde(default)]
    pub styles: BTreeMap<String, ComponentStyle>,
}

fn default_visible() -> Vec<ComponentKind> {
    vec![
        ComponentKind::Cpu,
        ComponentKind::Memory,
        ComponentKind::Gpu,
        ComponentKind::Network,
        ComponentKind::Disk,
    ]
}

impl Default for Components {
    fn default() -> Self {
        Self {
            visible: default_visible(),
            styles: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutConfig {
    #[serde(default = "default_template")]
    pub template: String,
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
}

fn default_template() -> String {
    "auto".to_string()
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            template: default_template(),
            overrides: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    Image,
    Font,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRef {
    pub path: String,
    pub kind: ResourceKind,
    #[serde(default)]
    pub sha256: Option<String>,
}

/// `theme.json` 的根结构。Schema v1 草案（见 `05`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeConfig {
    pub schema: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: Option<String>,
    pub version: String,
    pub palette: Palette,
    #[serde(default)]
    pub typography: Typography,
    #[serde(default)]
    pub background: Background,
    #[serde(default)]
    pub components: Components,
    #[serde(default)]
    pub layout: LayoutConfig,
    #[serde(default)]
    pub resources: Vec<ResourceRef>,
}

impl ThemeConfig {
    pub const SCHEMA_V1: &'static str = "hetao.theme/1";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_parses_and_serializes() {
        let c = Color::from_hex("#102030").expect("color");
        assert_eq!(c, Color::rgb(0x10, 0x20, 0x30));
        assert_eq!(c.to_hex(), "#102030");

        let c = Color::from_hex("#10203080").expect("color");
        assert_eq!(c.a, 0x80);
        assert_eq!(c.to_hex(), "#10203080");
    }

    #[test]
    fn invalid_color_is_rejected() {
        assert!(Color::from_hex("102030").is_err());
        assert!(Color::from_hex("#12345").is_err());
        assert!(Color::from_hex("#GGGGGG").is_err());
    }
}
