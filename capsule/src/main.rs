//! 核桃胶囊 — 电脑端入口。
//!
//! 只连接真实 NAS，不提供任何模拟/预览数据源。

#![windows_subsystem = "windows"]

mod app;
mod config;
mod data;
mod fmt;
mod font;
mod icons;
mod ui;

fn main() {
    // 单实例：已有胶囊在跑（含升级后旧版本仍在运行的情况）就不再起第二个，
    // 否则托盘里会冒出两个胶囊、两个都在收数据。
    if !acquire_single_instance() {
        return;
    }

    app::run(config::load());
}

/// 拿一个命名互斥体判断是否已有实例：已存在（ERROR_ALREADY_EXISTS）说明别人在跑。
/// 句柄本身不释放——它的生命周期就是进程生命周期，进程退出系统自动回收。
fn acquire_single_instance() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = "Local\\HetaoCapsule.SingleInstance\0"
        .encode_utf16()
        .collect();
    unsafe {
        match CreateMutexW(None, false, PCWSTR(name.as_ptr())) {
            Ok(_) => GetLastError() != ERROR_ALREADY_EXISTS,
            // 创建失败（极端情况）不拦启动，宁可多开也别打不开
            Err(_) => true,
        }
    }
}
