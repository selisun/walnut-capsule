//! 主题注册与切换。内置主题只读。

use std::collections::{BTreeMap, BTreeSet};

use screen_core::ThemeConfig;

use crate::error::ThemeError;

#[derive(Debug, Default)]
pub struct ThemeStore {
    themes: BTreeMap<String, ThemeConfig>,
    builtin: BTreeSet<String>,
    current: Option<String>,
}

impl ThemeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, theme: ThemeConfig, is_builtin: bool) -> Result<(), ThemeError> {
        let id = theme.id.clone();
        if self.themes.contains_key(&id) {
            return Err(ThemeError::InvalidField(format!("主题 id 重复：{id}")));
        }
        if is_builtin {
            self.builtin.insert(id.clone());
        }
        self.themes.insert(id.clone(), theme);
        if self.current.is_none() {
            self.current = Some(id);
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&ThemeConfig> {
        self.themes.get(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.themes.keys()
    }

    pub fn current_id(&self) -> Option<&str> {
        self.current.as_deref()
    }

    pub fn current(&self) -> Option<&ThemeConfig> {
        self.current.as_ref().and_then(|id| self.themes.get(id))
    }

    /// 切换前由调用方先 `validate` 新主题；这里只做注册表内切换。
    pub fn switch(&mut self, id: &str) -> Result<ThemeConfig, ThemeError> {
        let theme = self
            .themes
            .get(id)
            .cloned()
            .ok_or_else(|| ThemeError::ThemeNotFound(id.to_string()))?;
        self.current = Some(id.to_string());
        Ok(theme)
    }

    pub fn remove(&mut self, id: &str) -> Result<(), ThemeError> {
        if self.builtin.contains(id) {
            return Err(ThemeError::BuiltinReadOnly(id.to_string()));
        }
        if self.themes.remove(id).is_none() {
            return Err(ThemeError::ThemeNotFound(id.to_string()));
        }
        if self.current.as_deref() == Some(id) {
            self.current = self.themes.keys().next().cloned();
        }
        Ok(())
    }

    pub fn is_builtin(&self, id: &str) -> bool {
        self.builtin.contains(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::load_str;

    const BASE: &str = r##"{
        "schema":"hetao.theme/1","id":"t1","name":"主题一","version":"1.0.0",
        "palette":{"bg":"#101418","card":"#1B2028","text":"#E8EDF2","dim":"#8A94A6","accent":"#37D67A","warn":"#FFB020","danger":"#FF5A5F"}
    }"##;

    #[test]
    fn switch_between_registered_themes() {
        let theme = load_str(BASE).expect("theme");
        let mut store = ThemeStore::new();
        store.register(theme, false).expect("register");
        assert_eq!(store.current_id(), Some("t1"));

        let other = load_str(&BASE.replace("t1", "t2")).expect("theme2");
        store.register(other, false).expect("register2");
        store.switch("t2").expect("switch");
        assert_eq!(store.current_id(), Some("t2"));
        assert!(store.switch("missing").is_err());
    }

    #[test]
    fn builtin_theme_cannot_be_removed() {
        let theme = load_str(BASE).expect("theme");
        let mut store = ThemeStore::new();
        store.register(theme, true).expect("register");
        assert!(matches!(
            store.remove("t1"),
            Err(ThemeError::BuiltinReadOnly(_))
        ));
    }

    #[test]
    fn duplicate_id_is_rejected() {
        let theme = load_str(BASE).expect("theme");
        let mut store = ThemeStore::new();
        store.register(theme.clone(), false).expect("register");
        assert!(store.register(theme, false).is_err());
    }
}
