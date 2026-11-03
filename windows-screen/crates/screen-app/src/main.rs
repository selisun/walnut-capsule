//! `hetao-screen` 入口。第一阶段只做初始化/状态输出/安全退出。

use std::path::Path;
use std::process::ExitCode;

use screen_app::{cli, Command, Config, ScreenApp};
use screen_theme::load_str;

fn main() -> ExitCode {
    let args = std::env::args().skip(1);
    let command = match cli::parse(args) {
        Ok(command) => command,
        Err(err) => {
            eprintln!("参数错误：{err}");
            eprintln!("{}", cli::HELP);
            return ExitCode::from(2);
        }
    };

    let result = match command {
        Command::Help => {
            println!("{}", cli::HELP);
            Ok(())
        }
        Command::Version => {
            println!("hetao-screen {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::PrintConfig { config } => print_config(config.as_deref()),
        Command::CheckTheme { path } => check_theme(&path),
        Command::ListDevices => screen_app::device_cmd::list_devices(),
        Command::DeviceCheck => screen_app::device_cmd::device_check(),
        Command::WatchDevices { seconds } => screen_app::device_cmd::watch_devices(seconds),
        Command::SendFrame { pattern, repeat } => {
            screen_app::device_cmd::send_test_frame(&pattern, repeat)
        }
        Command::DeviceSession { seconds } => screen_app::device_cmd::device_session(seconds),
        Command::Run {
            config,
            run_seconds,
        } => run(config.as_deref(), run_seconds),
        Command::SetOrientation {
            orientation,
            config,
        } => set_direction(config.as_deref(), Some(&orientation), None),
        Command::SetFlip { flip, config } => set_direction(config.as_deref(), None, Some(&flip)),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn print_config(path: Option<&Path>) -> Result<(), String> {
    let resolved = path
        .map(Path::to_path_buf)
        .or_else(screen_app::default_config_path);
    let (config, used_defaults) = match &resolved {
        Some(path) => Config::load_or_default(path).map_err(|e| e.to_string())?,
        None => (Config::default(), true),
    };
    let text = toml::to_string_pretty(&config).map_err(|e| e.to_string())?;
    println!("# used_defaults = {used_defaults}");
    print!("{text}");
    Ok(())
}

fn check_theme(path: &Path) -> Result<(), String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("读取失败 {}: {e}", path.display()))?;
    let theme = load_str(&text).map_err(|e| e.to_string())?;
    println!(
        "OK: theme {} ({}) schema={} resources={}",
        theme.id,
        theme.name,
        theme.schema,
        theme.resources.len()
    );
    Ok(())
}

fn run(path: Option<&Path>, run_seconds: Option<u64>) -> Result<(), String> {
    let mut app = ScreenApp::init(path).map_err(|e| e.to_string())?;
    let options = screen_app::RunOptions {
        run_seconds,
        ..Default::default()
    };
    app.run_with_options(&options).map_err(|e| e.to_string())
}

fn set_direction(
    path: Option<&Path>,
    orientation: Option<&str>,
    flip: Option<&str>,
) -> Result<(), String> {
    let mut app = ScreenApp::init(path).map_err(|e| e.to_string())?;
    if let Some(orientation) = orientation {
        app.set_orientation(orientation)
            .map_err(|e| e.to_string())?;
    }
    if let Some(flip) = flip {
        app.set_flip(flip).map_err(|e| e.to_string())?;
    }
    println!(
        "OK: orientation={} flip={}",
        app.config().display.orientation,
        app.config().display.flip
    );
    Ok(())
}
