//! 极简日志：按 `logging.level` 过滤，控制台 + 可选文件落盘。
//!
//! 不引入第三方 crate；单行格式 `epoch.millis [LEVEL] message`。
//! 日志**不得**包含 Token/密码/Authorization，调用方自行保证。

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn parse(value: &str) -> Level {
        match value {
            "warn" => Level::Warn,
            "error" => Level::Error,
            _ => Level::Info,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Logger {
    level: Level,
    file: Option<PathBuf>,
}

impl Logger {
    pub fn new(level: &str, file: Option<PathBuf>) -> Self {
        Self {
            level: Level::parse(level),
            file,
        }
    }

    /// 仅记录 `level` 及更严重的日志。
    pub fn enabled(&self, level: Level) -> bool {
        level >= self.level
    }

    pub fn log(&self, level: Level, message: &str) {
        if !self.enabled(level) {
            return;
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let mut line = String::with_capacity(message.len() + 32);
        let _ = write!(
            line,
            "{}.{:03} [{}] {}",
            stamp.as_secs(),
            stamp.subsec_millis(),
            level.label(),
            message
        );
        // 控制台：stdout 给 info，stderr 给 warn/error（便于重定向分离）
        if matches!(level, Level::Info) {
            println!("{line}");
        } else {
            eprintln!("{line}");
        }
        if let Some(path) = self.file.as_ref() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut f| writeln!(f, "{line}"));
        }
    }

    pub fn info(&self, message: &str) {
        self.log(Level::Info, message);
    }

    pub fn warn(&self, message: &str) {
        self.log(Level::Warn, message);
    }

    pub fn error(&self, message: &str) {
        self.log(Level::Error, message);
    }
}

static GLOBAL: OnceLock<Logger> = OnceLock::new();

/// 初始化全局日志（幂等：重复调用保留第一次的设置）。
pub fn init(level: &str, file: Option<PathBuf>) {
    let _ = GLOBAL.set(Logger::new(level, file));
}

fn global() -> Option<&'static Logger> {
    GLOBAL.get()
}

pub fn info(message: &str) {
    match global() {
        Some(logger) => logger.info(message),
        None => println!("{message}"),
    }
}

pub fn warn(message: &str) {
    match global() {
        Some(logger) => logger.warn(message),
        None => eprintln!("{message}"),
    }
}

pub fn error(message: &str) {
    match global() {
        Some(logger) => logger.error(message),
        None => eprintln!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hetao-screen-log-{}-{}-{tag}.log",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn level_filter_suppresses_lower_levels() {
        let path = temp_log("filter");
        let logger = Logger::new("warn", Some(path.clone()));
        logger.info("这条不该落盘");
        logger.warn("这条应该落盘");
        logger.error("这条也应该落盘");

        let text = std::fs::read_to_string(&path).expect("log file");
        assert!(!text.contains("这条不该落盘"));
        assert!(text.contains("[WARN] 这条应该落盘"));
        assert!(text.contains("[ERROR] 这条也应该落盘"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn info_level_writes_everything() {
        let path = temp_log("info");
        let logger = Logger::new("info", Some(path.clone()));
        logger.info("hello");
        let text = std::fs::read_to_string(&path).expect("log file");
        assert!(text.contains("[INFO] hello"));
        assert!(logger.enabled(Level::Error));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_parent_directory_is_created() {
        let dir = std::env::temp_dir().join(format!("hetao-log-dir-{}", std::process::id()));
        let path = dir.join("nested").join("screen.log");
        let _ = std::fs::remove_dir_all(&dir);
        let logger = Logger::new("info", Some(path.clone()));
        logger.info("nested");
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn init_is_idempotent_and_unknown_level_defaults_to_info() {
        init("info", None);
        init("warn", None);
        let logger = Logger::new("unknown", None);
        assert!(logger.enabled(Level::Info), "未知级别按 info 处理");
        assert!(!Logger::new("error", None).enabled(Level::Warn));
    }
}
