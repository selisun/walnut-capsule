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
// B-10 P1：快照映射与 IPC 编码。P2 才接入管道，故先允许未使用告警。
#[allow(dead_code)]
mod screen_bridge;
mod screen_host;
mod screen_native;
mod ui;

fn main() {
    // 单实例：已有胶囊在跑（含升级后旧版本仍在运行的情况）就不再起第二个，
    // 否则托盘里会冒出两个胶囊、两个都在收数据。
    if !acquire_single_instance() {
        return;
    }

    let config = config::load();
    screen_native::log_startup(&config.screen);
    // 小屏脚本由客户端后台托管（无窗口 PowerShell），配置就绪后拉起（设置里「关闭小屏」后不再拉起）
    if config.screen.enabled && config.screen.engine == config::SCREEN_ENGINE_NATIVE {
        // B-10：native 引擎由胶囊负责写配置、拉起 hetao-screen.exe 并巡检；
        // exe 缺失/拉起失败时回退旧 PowerShell，保证小屏不至于全黑。
        screen_host::stop();
        if !screen_native::start(&config.screen) {
            screen_host::start();
        }
    } else if config.screen.enabled {
        // 从 native 切回 powershell 时，必须先收掉 native 进程，否则它继续占着串口
        screen_native::stop();
        screen_host::start();
    } else {
        // 上次是被「关闭小屏」关掉的：顺手清掉升级/异常退出残留的脚本，
        // 否则串口还被旧脚本占着，界面会显示「已连接」却关不掉
        screen_native::stop();
        screen_host::stop();
    }
    app::run(config);
    // 经由非 WM_DESTROY 的异常返回路径也收尾：先停推送（发 Bye），再收进程，避免残留。
    screen_bridge::stop();
    screen_native::stop();
    screen_host::stop();
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
