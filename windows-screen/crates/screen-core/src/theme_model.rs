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
        let bytes = s.as_bytes();
        // 按字节解析：先校验长度，再逐字节取十六进制位。
        // 不能用 &s[0..2] 切片，非 ASCII 输入会在非字符边界上 panic。
        if bytes.len() != 6 && bytes.len() != 8 {
            return Err("颜色长度必须为 6 或 8 位十六进制");
        }
        let mut c = [255u8; 4];
        for (i, slot) in c.iter_mut().take(bytes.len() / 2).enumerate() {
            let hi = hex_nibble(bytes[i * 2]).ok_or("颜色格式错误")?;
            let lo = hex_nibble(bytes[i * 2 + 1]).ok_or("颜色格式错误")?;
            *slot = hi * 16 + lo;
        }
        Ok(Self::new(c[0], c[1], c[2], c[3]))
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

/// 单个十六进制字符 → 0..=15；非 ASCII 十六进制位返回 None。
fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
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
    /// 按硬件品牌选图：键为 `{cpu}-{gpu}`（amd/intel/unknown × nvidia/amd/intel/unknown），
    /// 值为图片文件名。命中时优先于 `image`，用于官方皮肤的两处品牌 logo。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variants: BTreeMap<String, String>,
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
            variants: BTreeMap::new(),
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

/// 数值槽的取值来源。只允许已采集的指标，缺失时整槽留空（不填 0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayMetric {
    Cpu,
    CpuTemp,
    Gpu,
    GpuTemp,
    Memory,
    MemoryUsed,
    MemoryTotal,
    NetworkDown,
    NetworkUp,
    DiskRead,
    DiskWrite,
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SlotAlign {
    #[default]
    Left,
    Right,
    Center,
}

/// 底图上的一个数值槽：绝对像素矩形 + 字号 + 对齐。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlaySlot {
    pub metric: OverlayMetric,
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub w: f32,
    #[serde(default = "default_slot_size")]
    pub size: f32,
    #[serde(default)]
    pub align: SlotAlign,
    /// 未填时用主题 `palette.text`。
    #[serde(default)]
    pub color: Option<Color>,
    /// 数值后缀（底图没画单位时用）。
    #[serde(default)]
    pub suffix: Option<String>,
    /// 小数位；百分比类固定 0 位，容量/速率类用 1 位。
    #[serde(default = "default_slot_decimals")]
    pub decimals: u8,
    /// 速率单位：`auto`（B/s、KB/s、MB/s 自动，默认）或 `mb`（统一换算成 MB/s，
    /// 用于底图把单位固定印成 `MB/s` 的官方皮肤）。
    #[serde(default)]
    pub rate_unit: RateUnit,
}

/// 速率类指标的单位口径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateUnit {
    /// 自动选择 B/s、KB/s、MB/s。
    #[default]
    Auto,
    /// 一律按 MB/s 输出纯数字（单位由底图提供），小数位固定为 `decimals`。
    Mb,
    /// 一律按 MB/s 输出，小数位按量级自适应，使字符串长度稳定在 5 个字符左右：
    /// `<10` 三位、`10–99` 两位、`100–999` 一位、`≥1000` 整数；
    /// 非零但不足 0.001 时显示 `0.001`。
    MbFit,
}

fn default_slot_size() -> f32 {
    24.0
}

fn default_slot_decimals() -> u8 {
    0
}

/// 「图片 + 数值」主题：底图已画好卡片/图标/单位，这里只声明数值贴在哪。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Overlay {
    #[serde(default)]
    pub slots: Vec<OverlaySlot>,
    /// 按屏幕方向覆盖槽位：键为 `landscape` / `portrait`，命中时优先于 `slots`。
    /// 官方皮肤横竖两套版式的数值位置不同，用这个字段在同一个主题里分别声明。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variants: BTreeMap<String, Vec<OverlaySlot>>,
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
    /// 版式风格：`legacy-single`（单列卡片+圆环）| `legacy-dual`（左 NAS / 右电脑）。
    #[serde(default)]
    pub render: Option<String>,
    /// 「图片 + 数值」：底图 + 数值槽；存在时只贴数值，不画任何图形。
    #[serde(default)]
    pub overlay: Option<Overlay>,
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

    #[test]
    fn non_ascii_color_is_rejected_without_panic() {
        // 6 字节但不是 6 个 ASCII 字符：旧实现会在 &s[0..2] 处按非字符边界切片而 panic
        assert!(Color::from_hex("#aééB").is_err());
        assert!(Color::from_hex("#内存").is_err());
    }
}
