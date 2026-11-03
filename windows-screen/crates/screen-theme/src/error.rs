//! 主题错误。错误信息只描述字段/原因，不含任何凭据。

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeError {
    TooLarge { max: usize, got: usize },
    Parse(String),
    UnsupportedSchema(String),
    InvalidField(String),
    UnsafePath(String),
    ForbiddenResource(String),
    MissingHash(String),
    MissingResource(String),
    VersionMismatch(String),
    ThemeNotFound(String),
    BuiltinReadOnly(String),
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThemeError::TooLarge { max, got } => write!(f, "主题文件过大：{got} > {max}"),
            ThemeError::Parse(msg) => write!(f, "主题 JSON 解析失败：{msg}"),
            ThemeError::UnsupportedSchema(schema) => write!(f, "不支持的主题 Schema：{schema}"),
            ThemeError::InvalidField(name) => write!(f, "主题字段非法：{name}"),
            ThemeError::UnsafePath(path) => write!(f, "资源路径不安全：{path}"),
            ThemeError::ForbiddenResource(path) => write!(f, "禁止的资源类型：{path}"),
            ThemeError::MissingHash(path) => write!(f, "资源缺少 sha256：{path}"),
            ThemeError::MissingResource(path) => write!(f, "引用的资源不存在：{path}"),
            ThemeError::VersionMismatch(v) => write!(f, "主题版本格式非法：{v}"),
            ThemeError::ThemeNotFound(id) => write!(f, "主题不存在：{id}"),
            ThemeError::BuiltinReadOnly(id) => write!(f, "内置主题只读：{id}"),
        }
    }
}

impl std::error::Error for ThemeError {}
