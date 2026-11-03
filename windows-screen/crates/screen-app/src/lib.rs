//! # screen-app
//!
//! 小屏独立进程的装配层：配置加载、模块初始化、生命周期、退出释放。

pub mod app;
pub mod cli;
pub mod config;
pub mod device_cmd;
pub mod orientation;
pub mod pattern;
pub mod runtime;
pub mod session;

pub use app::{default_config_path, AppError, AppState, ScreenApp};
pub use cli::{Command, HELP};
pub use config::Config;
pub use orientation::{Direction, DirectionError, Flip, FramePlan, LogicalOrientation};
pub use runtime::{
    run_loop, run_production, DeviceProvider, EventPoller, IpcPoller, RunOptions, RunReport,
    RuntimeClock,
};
pub use session::{DeviceSession, SessionError, SessionState};
