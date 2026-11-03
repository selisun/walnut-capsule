//! 极简命令行解析（不引入 clap，遵守依赖审批）。

use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run {
        config: Option<PathBuf>,
        run_seconds: Option<u64>,
    },
    SetOrientation {
        orientation: String,
        config: Option<PathBuf>,
    },
    SetFlip {
        flip: String,
        config: Option<PathBuf>,
    },
    PrintConfig {
        config: Option<PathBuf>,
    },
    CheckTheme {
        path: PathBuf,
    },
    ListDevices,
    DeviceCheck,
    WatchDevices {
        seconds: u64,
    },
    SendFrame {
        pattern: String,
        repeat: u32,
    },
    DeviceSession {
        seconds: u64,
    },
    Version,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    MissingValue(&'static str),
    UnknownArgument(String),
    UnexpectedArgument(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::MissingValue(flag) => write!(f, "{flag} 缺少参数"),
            CliError::UnknownArgument(arg) => write!(f, "未知参数：{arg}"),
            CliError::UnexpectedArgument(arg) => write!(f, "多余参数：{arg}"),
        }
    }
}

impl std::error::Error for CliError {}

pub fn parse<I, S>(args: I) -> Result<Command, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut config: Option<PathBuf> = None;
    let mut theme: Option<PathBuf> = None;
    let mut print_config = false;
    let mut version = false;
    let mut help = false;
    let mut list_devices = false;
    let mut device_check = false;
    let mut watch: Option<u64> = None;
    let mut send_frame: Option<String> = None;
    let mut session: Option<u64> = None;
    let mut repeat: u32 = 1;
    let mut run_seconds: Option<u64> = None;
    let mut set_orientation: Option<String> = None;
    let mut set_flip: Option<String> = None;

    let mut iter = args.into_iter().map(Into::into);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--config" => {
                let value = iter.next().ok_or(CliError::MissingValue("--config"))?;
                config = Some(PathBuf::from(value));
            }
            "--check-theme" => {
                let value = iter.next().ok_or(CliError::MissingValue("--check-theme"))?;
                theme = Some(PathBuf::from(value));
            }
            "--print-config" => print_config = true,
            "--list-devices" => list_devices = true,
            "--device-check" => device_check = true,
            "--watch-devices" => {
                let value = iter
                    .next()
                    .ok_or(CliError::MissingValue("--watch-devices"))?;
                watch = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| CliError::MissingValue("--watch-devices 需要秒数"))?,
                );
            }
            "--send-frame" => {
                let value = iter.next().ok_or(CliError::MissingValue("--send-frame"))?;
                send_frame = Some(value);
            }
            "--repeat" => {
                let value = iter.next().ok_or(CliError::MissingValue("--repeat"))?;
                repeat = value
                    .parse::<u32>()
                    .map_err(|_| CliError::MissingValue("--repeat 需要 1..=10"))?;
            }
            "--device-session" => {
                let value = iter
                    .next()
                    .ok_or(CliError::MissingValue("--device-session"))?;
                session = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| CliError::MissingValue("--device-session 需要秒数"))?,
                );
            }
            "--run-seconds" => {
                let value = iter.next().ok_or(CliError::MissingValue("--run-seconds"))?;
                let seconds = value
                    .parse::<u64>()
                    .map_err(|_| CliError::MissingValue("--run-seconds 需要秒数"))?;
                if !(1..=86_400).contains(&seconds) {
                    return Err(CliError::UnexpectedArgument(
                        "--run-seconds 必须在 1..=86400".to_string(),
                    ));
                }
                run_seconds = Some(seconds);
            }
            "--set-orientation" => {
                let value = iter
                    .next()
                    .ok_or(CliError::MissingValue("--set-orientation"))?;
                if !["portrait", "landscape"].contains(&value.as_str()) {
                    return Err(CliError::UnexpectedArgument(
                        "--set-orientation 只允许 portrait|landscape".to_string(),
                    ));
                }
                set_orientation = Some(value);
            }
            "--set-flip" => {
                let value = iter.next().ok_or(CliError::MissingValue("--set-flip"))?;
                if !["normal", "flipped"].contains(&value.as_str()) {
                    return Err(CliError::UnexpectedArgument(
                        "--set-flip 只允许 normal|flipped".to_string(),
                    ));
                }
                set_flip = Some(value);
            }
            "--version" | "-V" => version = true,
            "--help" | "-h" => help = true,
            other => return Err(CliError::UnknownArgument(other.to_string())),
        }
    }

    if help {
        return Ok(Command::Help);
    }
    if version {
        return Ok(Command::Version);
    }

    let has_device_action = list_devices
        || device_check
        || watch.is_some()
        || send_frame.is_some()
        || session.is_some();

    if set_orientation.is_some() || set_flip.is_some() {
        if set_orientation.is_some() && set_flip.is_some() {
            return Err(CliError::UnexpectedArgument(
                "--set-orientation 与 --set-flip 一次只能设置一个".to_string(),
            ));
        }
        if has_device_action || theme.is_some() || print_config || run_seconds.is_some() {
            return Err(CliError::UnexpectedArgument(
                "方向设置命令不能与其他命令同时使用".to_string(),
            ));
        }
        if let Some(orientation) = set_orientation {
            return Ok(Command::SetOrientation {
                orientation,
                config,
            });
        }
        return Ok(Command::SetFlip {
            flip: set_flip.expect("已检查存在"),
            config,
        });
    }

    if run_seconds.is_some() && (has_device_action || theme.is_some() || print_config) {
        return Err(CliError::UnexpectedArgument(
            "--run-seconds 只能用于默认运行命令".to_string(),
        ));
    }

    if list_devices || device_check || watch.is_some() || send_frame.is_some() || session.is_some()
    {
        let actions = [
            list_devices,
            device_check,
            watch.is_some(),
            send_frame.is_some(),
            session.is_some(),
        ]
        .iter()
        .filter(|v| **v)
        .count();
        if actions > 1 {
            return Err(CliError::UnexpectedArgument(
                "设备命令一次只能执行一个".to_string(),
            ));
        }
        if theme.is_some() || print_config {
            return Err(CliError::UnexpectedArgument(
                "设备命令不能与 --check-theme/--print-config 同时使用".to_string(),
            ));
        }
        if list_devices {
            return Ok(Command::ListDevices);
        }
        if device_check {
            return Ok(Command::DeviceCheck);
        }
        if let Some(seconds) = watch {
            return Ok(Command::WatchDevices { seconds });
        }
        if let Some(pattern) = send_frame {
            if !(1..=10).contains(&repeat) {
                return Err(CliError::UnexpectedArgument(
                    "--repeat 必须在 1..=10".to_string(),
                ));
            }
            return Ok(Command::SendFrame { pattern, repeat });
        }
        if let Some(seconds) = session {
            return Ok(Command::DeviceSession { seconds });
        }
    }
    if let Some(path) = theme {
        if print_config || config.is_some() {
            return Err(CliError::UnexpectedArgument(format!(
                "--check-theme 不能与 {} 同时使用",
                if print_config {
                    "--print-config"
                } else {
                    "--config"
                }
            )));
        }
        return Ok(Command::CheckTheme { path });
    }
    if print_config {
        return Ok(Command::PrintConfig { config });
    }
    Ok(Command::Run {
        config,
        run_seconds,
    })
}

pub const HELP: &str = "\
hetao-screen — 核桃胶囊小屏进程

用法:
  hetao-screen [--config <path>] [--run-seconds <N>]  启动常驻主循环（无设备/无客户端时等待，N 秒后退出）
  hetao-screen --set-orientation <portrait|landscape> [--config <path>]  写配置后退出
  hetao-screen --set-flip <normal|flipped> [--config <path>]              写配置后退出
  hetao-screen --print-config [--config <path>]    打印生效配置
  hetao-screen --check-theme <theme.json>          校验主题 JSON
  hetao-screen --list-devices                      枚举 COMPORT 候选（不打开）
  hetao-screen --device-check                      打开/参数校验/初始化基线命令（P-1）
  hetao-screen --watch-devices <seconds>            CM 通知监听（不打开设备，5..=300）
  hetao-screen --send-frame <bars|gradient|solid> [--repeat N]   发送测试帧（P-2/P-3）
  hetao-screen --device-session <seconds>          连续发送并监测断开/恢复（5..=600）
  hetao-screen --version
  hetao-screen --help

注意：只支持 3.5 英寸已验证基线；不发 0x79、不运行官方软件、不抓包。
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_run() {
        let cmd = parse(std::iter::empty::<String>()).expect("cmd");
        assert_eq!(
            cmd,
            Command::Run {
                config: None,
                run_seconds: None
            }
        );
    }

    #[test]
    fn run_seconds_is_parsed() {
        let cmd = parse(["--run-seconds", "10"]).expect("cmd");
        assert_eq!(
            cmd,
            Command::Run {
                config: None,
                run_seconds: Some(10)
            }
        );
        assert!(parse(["--run-seconds", "0"]).is_err());
        assert!(parse(["--run-seconds", "999999"]).is_err());
    }

    #[test]
    fn direction_settings_parse() {
        assert_eq!(
            parse(["--set-orientation", "landscape"]).expect("cmd"),
            Command::SetOrientation {
                orientation: "landscape".to_string(),
                config: None
            }
        );
        assert_eq!(
            parse(["--config", "c.toml", "--set-flip", "flipped"]).expect("cmd"),
            Command::SetFlip {
                flip: "flipped".to_string(),
                config: Some(PathBuf::from("c.toml"))
            }
        );
        assert!(parse(["--set-orientation", "sideways"]).is_err());
        assert!(parse(["--set-flip", "mirror"]).is_err());
        assert!(parse(["--set-orientation", "portrait", "--set-flip", "normal"]).is_err());
        assert!(parse(["--set-orientation", "portrait", "--run-seconds", "10"]).is_err());
        assert!(parse(["--run-seconds", "10", "--list-devices"]).is_err());
    }

    #[test]
    fn config_path_is_parsed() {
        let cmd = parse(["--config", "a.toml", "--print-config"]).expect("cmd");
        assert_eq!(
            cmd,
            Command::PrintConfig {
                config: Some(PathBuf::from("a.toml"))
            }
        );
    }

    #[test]
    fn check_theme_requires_path() {
        assert_eq!(
            parse(["--check-theme", "t.json"]).expect("cmd"),
            Command::CheckTheme {
                path: PathBuf::from("t.json")
            }
        );
        assert!(parse(["--check-theme"]).is_err());
    }

    #[test]
    fn unknown_flag_is_rejected() {
        assert!(parse(["--nope"]).is_err());
    }

    #[test]
    fn version_and_help() {
        assert_eq!(parse(["--version"]).expect("cmd"), Command::Version);
        assert_eq!(parse(["--help"]).expect("cmd"), Command::Help);
    }

    #[test]
    fn device_commands_parse() {
        assert_eq!(
            parse(["--list-devices"]).expect("cmd"),
            Command::ListDevices
        );
        assert_eq!(
            parse(["--device-check"]).expect("cmd"),
            Command::DeviceCheck
        );
        assert_eq!(
            parse(["--watch-devices", "10"]).expect("cmd"),
            Command::WatchDevices { seconds: 10 }
        );
        assert_eq!(
            parse(["--send-frame", "bars", "--repeat", "2"]).expect("cmd"),
            Command::SendFrame {
                pattern: "bars".to_string(),
                repeat: 2
            }
        );
        assert_eq!(
            parse(["--device-session", "30"]).expect("cmd"),
            Command::DeviceSession { seconds: 30 }
        );
    }

    #[test]
    fn conflicting_device_commands_are_rejected() {
        assert!(parse(["--list-devices", "--device-check"]).is_err());
        assert!(parse(["--send-frame", "bars", "--repeat", "99"]).is_err());
        assert!(parse(["--send-frame", "bars", "--print-config"]).is_err());
    }
}
