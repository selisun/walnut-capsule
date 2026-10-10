//! 核桃胶囊 — 电脑端入口。
//! 命令行参数：
//!   --config <path>    指定配置文件
//!   --fake             强制使用 FakeSource（不连 NAS）

#![windows_subsystem = "windows"]

mod app;
mod config;
mod data;
mod fmt;
mod font;
mod icons;
mod ui;

use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut cfg = config::load();
    let mut force_fake = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--config" if i + 1 < args.len() => {
                // 暂不支持自定义路径（存读固定）
                i += 2;
            }
            "--fake" => {
                force_fake = true;
                i += 1;
            }
            _ => i += 1,
        }
    }
    if force_fake {
        cfg.nas.source = "fake".into();
    }

    app::run(cfg);
}
