//! # screen-theme
//!
//! 声明式 JSON 主题：加载、Schema v1 校验、资源路径与类型检查、注册/切换。
//!
//! 禁止执行任何用户提供的 PowerShell/EXE/DLL/脚本；资源只允许图片与字体。

pub mod error;
pub mod loader;
pub mod store;

pub use error::ThemeError;
pub use loader::{load_dir, load_str, validate, MAX_THEME_BYTES};
pub use store::ThemeStore;
