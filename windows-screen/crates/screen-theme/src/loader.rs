//! 主题加载与校验。

use std::collections::HashSet;

use screen_core::{ResourceKind, ThemeConfig};

use crate::error::ThemeError;

pub const MAX_THEME_BYTES: usize = 2 * 1024 * 1024;
const MAX_TEXT_FIELD: usize = 64;
const MAX_PATH_LEN: usize = 200;

const PALETTE_KEYS: [&str; 7] = ["bg", "card", "text", "dim", "accent", "warn", "danger"];
const TEMPLATE_NAMES: [&str; 5] = [
    "portrait-2col",
    "landscape-4col",
    "wide-6col",
    "square-2x2",
    "generic-4col",
];

/// 从字符串加载并校验主题。
pub fn load_str(json: &str) -> Result<ThemeConfig, ThemeError> {
    if json.len() > MAX_THEME_BYTES {
        return Err(ThemeError::TooLarge {
            max: MAX_THEME_BYTES,
            got: json.len(),
        });
    }
    let theme: ThemeConfig =
        serde_json::from_str(json).map_err(|e| ThemeError::Parse(e.to_string()))?;
    validate(&theme)?;
    Ok(theme)
}

pub fn validate(theme: &ThemeConfig) -> Result<(), ThemeError> {
    if theme.schema != ThemeConfig::SCHEMA_V1 {
        return Err(ThemeError::UnsupportedSchema(theme.schema.clone()));
    }
    if theme.id.trim().is_empty() || theme.id.len() > MAX_TEXT_FIELD {
        return Err(ThemeError::InvalidField("id".to_string()));
    }
    if !theme
        .id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(ThemeError::InvalidField("id 字符集".to_string()));
    }
    if theme.name.trim().is_empty() {
        return Err(ThemeError::InvalidField("name".to_string()));
    }
    if !valid_semver(&theme.version) {
        return Err(ThemeError::VersionMismatch(theme.version.clone()));
    }

    let typo = &theme.typography;
    if typo.family.trim().is_empty() || typo.family.len() > MAX_TEXT_FIELD {
        return Err(ThemeError::InvalidField("typography.family".to_string()));
    }
    if !typo.base_px.is_finite() || !(6.0..=48.0).contains(&typo.base_px) {
        return Err(ThemeError::InvalidField("typography.base_px".to_string()));
    }
    if !typo.scale.is_finite() || !(0.5..=3.0).contains(&typo.scale) {
        return Err(ThemeError::InvalidField("typography.scale".to_string()));
    }
    for (key, weight) in &typo.weights {
        if !(100..=900).contains(weight) || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(ThemeError::InvalidField("typography.weights".to_string()));
        }
    }

    let bg = &theme.background;
    if !bg.dim.is_finite() || !(0.0..=1.0).contains(&bg.dim) {
        return Err(ThemeError::InvalidField("background.dim".to_string()));
    }
    if let Some(path) = &bg.image {
        validate_path(path)?;
        let found = theme
            .resources
            .iter()
            .any(|r| r.kind == ResourceKind::Image && r.path == *path);
        if !found {
            return Err(ThemeError::MissingResource(path.clone()));
        }
    }

    let mut seen = HashSet::new();
    for kind in &theme.components.visible {
        if !seen.insert(*kind) {
            return Err(ThemeError::InvalidField(format!(
                "components.visible 重复：{}",
                kind.as_str()
            )));
        }
    }
    if theme.components.visible.is_empty() {
        return Err(ThemeError::InvalidField("components.visible".to_string()));
    }
    for (name, style) in &theme.components.styles {
        if !theme.components.visible.iter().any(|k| k.as_str() == name) {
            return Err(ThemeError::InvalidField(format!(
                "components.styles 引用了未显示组件：{name}"
            )));
        }
        if let Some(accent) = &style.accent {
            if !PALETTE_KEYS.contains(&accent.as_str()) {
                return Err(ThemeError::InvalidField(format!(
                    "components.styles.{name}.accent"
                )));
            }
        }
    }

    let mut paths = HashSet::new();
    for resource in &theme.resources {
        validate_path(&resource.path)?;
        if !paths.insert(resource.path.clone()) {
            return Err(ThemeError::InvalidField(format!(
                "资源路径重复：{}",
                resource.path
            )));
        }
        validate_extension(resource)?;
        match &resource.sha256 {
            Some(hash) if is_sha256(hash) => {}
            _ => return Err(ThemeError::MissingHash(resource.path.clone())),
        }
    }

    if theme.layout.template != "auto" && !TEMPLATE_NAMES.contains(&theme.layout.template.as_str())
    {
        return Err(ThemeError::InvalidField("layout.template".to_string()));
    }
    for (profile, template) in &theme.layout.overrides {
        if profile.trim().is_empty() {
            return Err(ThemeError::InvalidField("layout.overrides 键".to_string()));
        }
        if !TEMPLATE_NAMES.contains(&template.as_str()) {
            return Err(ThemeError::InvalidField(format!(
                "layout.overrides[{profile}]"
            )));
        }
    }

    Ok(())
}

fn valid_semver(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())
}

pub fn is_sha256(hash: &str) -> bool {
    hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())
}

fn validate_path(path: &str) -> Result<(), ThemeError> {
    if path.is_empty() || path.len() > MAX_PATH_LEN {
        return Err(ThemeError::UnsafePath(path.to_string()));
    }
    if path.starts_with('/') || path.starts_with('\\') || path.contains(':') {
        return Err(ThemeError::UnsafePath(path.to_string()));
    }
    if path.contains('\\') {
        return Err(ThemeError::UnsafePath(path.to_string()));
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains('\0'))
    {
        return Err(ThemeError::UnsafePath(path.to_string()));
    }
    Ok(())
}

fn validate_extension(resource: &screen_core::ResourceRef) -> Result<(), ThemeError> {
    let ext = resource
        .path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let forbidden = [
        "ps1", "exe", "dll", "bat", "cmd", "js", "vbs", "scr", "msi", "com", "cpl", "jar",
    ];
    if forbidden.contains(&ext.as_str()) {
        return Err(ThemeError::ForbiddenResource(resource.path.clone()));
    }
    let allowed = match resource.kind {
        ResourceKind::Image => ["png", "jpg", "jpeg", "bmp"].as_slice(),
        ResourceKind::Font => ["ttf", "otf", "woff2"].as_slice(),
    };
    if !allowed.contains(&ext.as_str()) {
        return Err(ThemeError::ForbiddenResource(resource.path.clone()));
    }
    Ok(())
}

/// 内置组件占位，供未来默认主题扩展时复用。
pub fn default_visible_components() -> Vec<screen_core::ComponentKind> {
    screen_core::Components::default().visible
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r##"{
        "schema":"hetao.theme/1",
        "id":"test",
        "name":"测试主题",
        "version":"1.0.0",
        "palette":{
            "bg":"#101418","card":"#1B2028","text":"#E8EDF2",
            "dim":"#8A94A6","accent":"#37D67A","warn":"#FFB020","danger":"#FF5A5F"
        }
    }"##;

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn with_resource(path: &str, kind: &str, hash: Option<&str>) -> String {
        let hash_field = match hash {
            Some(h) => format!(r#","sha256":"{h}""#),
            None => String::new(),
        };
        format!(
            r##"{{
                "schema":"hetao.theme/1","id":"res","name":"资源主题","version":"1.0.0",
                "palette":{{"bg":"#101418","card":"#1B2028","text":"#E8EDF2","dim":"#8A94A6","accent":"#37D67A","warn":"#FFB020","danger":"#FF5A5F"}},
                "background":{{"image":"{path}"}},
                "resources":[{{"path":"{path}","kind":"{kind}"{hash_field}}}]
            }}"##
        )
    }

    #[test]
    fn minimal_theme_is_valid() {
        let theme = load_str(BASE).expect("theme");
        assert_eq!(theme.id, "test");
        assert_eq!(theme.components.visible.len(), 5);
    }

    #[test]
    fn unknown_top_level_field_is_rejected() {
        let bad = BASE.replace("{", "{\"extra\":1,");
        assert!(matches!(load_str(&bad), Err(ThemeError::Parse(_))));
    }

    #[test]
    fn unsupported_schema_is_rejected() {
        let bad = BASE.replace("hetao.theme/1", "hetao.theme/2");
        assert!(matches!(
            load_str(&bad),
            Err(ThemeError::UnsupportedSchema(_))
        ));
    }

    #[test]
    fn invalid_color_is_rejected_at_parse() {
        let bad = BASE.replace("#101418", "red");
        assert!(matches!(load_str(&bad), Err(ThemeError::Parse(_))));
    }

    #[test]
    fn absolute_and_traversal_paths_are_rejected() {
        let bad = with_resource("/etc/passwd", "image", Some(HASH));
        assert!(matches!(load_str(&bad), Err(ThemeError::UnsafePath(_))));
        let bad = with_resource("../secret.png", "image", Some(HASH));
        assert!(matches!(load_str(&bad), Err(ThemeError::UnsafePath(_))));
        let bad = with_resource("C:/tmp/x.png", "image", Some(HASH));
        assert!(matches!(load_str(&bad), Err(ThemeError::UnsafePath(_))));
    }

    #[test]
    fn script_resources_are_forbidden() {
        let bad = with_resource("assets/theme.ps1", "image", Some(HASH));
        assert!(matches!(
            load_str(&bad),
            Err(ThemeError::ForbiddenResource(_))
        ));
    }

    #[test]
    fn image_extension_is_enforced() {
        let bad = with_resource("assets/bg.svg", "image", Some(HASH));
        assert!(matches!(
            load_str(&bad),
            Err(ThemeError::ForbiddenResource(_))
        ));
    }

    #[test]
    fn missing_hash_is_rejected() {
        let bad = with_resource("assets/bg.png", "image", None);
        assert!(matches!(load_str(&bad), Err(ThemeError::MissingHash(_))));
    }

    #[test]
    fn background_not_registered_is_rejected() {
        let json = BASE.replace(
            "\"palette\"",
            "\"background\":{\"image\":\"assets/bg.png\"},\"resources\":[],\"palette\"",
        );
        assert!(matches!(
            load_str(&json),
            Err(ThemeError::MissingResource(_))
        ));
    }

    #[test]
    fn duplicate_visible_component_is_rejected() {
        let json = BASE.replace(
            "\"palette\"",
            "\"components\":{\"visible\":[\"cpu\",\"cpu\"]},\"palette\"",
        );
        assert!(matches!(load_str(&json), Err(ThemeError::InvalidField(_))));
    }

    #[test]
    fn unknown_template_is_rejected() {
        let json = BASE.replace(
            "\"palette\"",
            "\"layout\":{\"template\":\"made-up\"},\"palette\"",
        );
        assert!(matches!(load_str(&json), Err(ThemeError::InvalidField(_))));
    }

    #[test]
    fn valid_resource_theme_loads() {
        let good = with_resource("assets/bg.png", "image", Some(HASH));
        let theme = load_str(&good).expect("theme");
        assert_eq!(theme.resources.len(), 1);
        assert_eq!(theme.background.image.as_deref(), Some("assets/bg.png"));
    }

    #[test]
    fn default_visible_components_are_non_empty() {
        assert_eq!(default_visible_components().len(), 5);
    }
}
