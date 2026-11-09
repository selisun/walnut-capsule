//! 生产主循环：设备发现/连接/重连、IPC 快照接收、渲染发送、占位画面、优雅退出。
//!
//! 外部依赖通过 trait 注入，便于用 mock 做状态转移测试；生产实现位于本文件底部。

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use screen_adapter_api::{AdapterError, DeviceAdapter, DeviceInfo};
use screen_core::{Frame, Orientation, ScreenProfile};
use screen_data::DataState;
use screen_ipc::{ConnectionState, Message};
use screen_transport_cdc::DeviceEvent;

use crate::app::{AppError, ScreenApp};
use crate::dirty::{plan_send, FrameSend};
use crate::session::{DeviceSession, SessionError};

/// 3.5 英寸物理基线恒为 320×480 / Portrait。
pub const PHYSICAL_WIDTH: u32 = 320;
pub const PHYSICAL_HEIGHT: u32 = 480;

#[derive(Debug, Clone, PartialEq)]
pub struct RunOptions {
    pub physical: ScreenProfile,
    /// `None` = 无限运行；`Some(n)` = n 秒后正常退出。
    pub run_seconds: Option<u64>,
    /// 主循环节拍（默认 1s，与 `display.refresh_secs` 对齐）。
    pub tick_ms: u64,
    /// 设备重连退避，必须 ≥5s。
    pub retry_ms: u64,
    /// 每个节拍最多处理的 IPC 消息数。
    pub max_ipc_messages_per_tick: usize,
    /// 每个节拍最多处理的设备事件数。
    pub max_events_per_tick: usize,
    /// 设备状态上报文件；None = 不写（测试用）。
    pub status_file: Option<PathBuf>,
    /// 周期性整帧校准间隔（ms）。
    ///
    /// 局部刷新依赖「发送成功 = 设备显示正确」。设备端一旦丢掉一帧，差异此后
    /// 不会再被重发，画面会长期停在旧值。每隔这个间隔强制发一次整帧，
    /// 把设备内容与本地缓存重新对齐。0 = 关闭校准。
    pub full_refresh_ms: u64,
    /// 清屏同步帧发送后、首帧内容前的间隔（ms）。
    ///
    /// 设备单缓冲，连续两整帧背靠背下发时，最后一分块（4096 字节，约 6 行）
    /// 常因设备还在刷新上一帧而被丢弃，表现为屏幕底部残留一小块清屏黑。
    /// 留出一帧的显示时间再做后续发送。0 = 不等待（测试用）。
    pub sync_settle_ms: u64,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            physical: ScreenProfile::new(
                "3.5-physical",
                PHYSICAL_WIDTH,
                PHYSICAL_HEIGHT,
                Orientation::Portrait,
                true,
            )
            .expect("固定物理 Profile 必须有效"),
            run_seconds: None,
            tick_ms: 1_000,
            retry_ms: 5_000,
            max_ipc_messages_per_tick: 8,
            max_events_per_tick: 4,
            status_file: Some(status_path()),
            full_refresh_ms: DEFAULT_FULL_REFRESH_MS,
            sync_settle_ms: DEFAULT_SYNC_SETTLE_MS,
        }
    }
}

/// 默认整帧校准间隔：5 分钟。
pub const DEFAULT_FULL_REFRESH_MS: u64 = 300_000;

/// 清屏同步帧后的默认间隔：200ms（约 3~6 帧显示时间）。
pub const DEFAULT_SYNC_SETTLE_MS: u64 = 200;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunReport {
    pub ticks: u64,
    pub frames_sent: u64,
    pub send_failures: u64,
    pub render_failures: u64,
    pub connect_attempts: u64,
    pub connect_successes: u64,
    pub ipc_messages: u64,
    pub ipc_rejects: u64,
    pub stale_frames: u64,
    pub waiting_frames: u64,
    /// 只发了脏矩形（未发整帧）的次数。
    pub frames_partial: u64,
    /// 与上一帧完全相同、未发送的次数。
    pub frames_skipped: u64,
    /// 连接建立后下发的清屏同步帧次数（每次连接恰好一帧，失败不计）。
    pub sync_frames: u64,
}

/// 时钟与退出信号。
pub trait RuntimeClock {
    fn now_ms(&mut self) -> i64;
    fn sleep_ms(&mut self, ms: u64);
    fn should_stop(&self) -> bool;
}

/// 设备发现/打开注入点。
pub trait DeviceProvider {
    /// `Ok(None)` 表示未发现设备（不是错误）；`Err` 表示打开/枚举失败。
    fn discover(&mut self) -> Result<Option<DeviceInfo>, String>;
    fn open(&mut self, info: &DeviceInfo) -> Result<DeviceSession<dyn DeviceAdapter>, String>;
}

/// IPC 轮询注入点。
pub trait IpcPoller {
    fn poll_message(&mut self) -> Result<Option<Message>, String>;
    fn state(&self) -> ConnectionState;
    fn close(&mut self);
}

/// 设备事件注入点。
pub trait EventPoller {
    fn try_recv(&mut self) -> Option<DeviceEvent>;
}

/// 设备状态文件：胶囊按它显示“已连接 / 未连接”，并决定是否弹断开提示。
fn status_path() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hetao-capsule")
        .join("screen")
        .join("status.json")
}

fn status_json(connected: bool, updated_ms: u64, reason: &str) -> String {
    format!(
        "{{\"connected\":{},\"updated_ms\":{},\"reason\":\"{}\"}}",
        connected, updated_ms, reason
    )
}

fn write_device_status(path: Option<&std::path::Path>, connected: bool, reason: &str) {
    let Some(path) = path else {
        return;
    };
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let _ = std::fs::write(path, status_json(connected, ms, reason));
}

/// 主循环：直到 `run_seconds` 到期或 `clock.should_stop()`。
pub fn run_loop(
    app: &mut ScreenApp,
    provider: &mut dyn DeviceProvider,
    mut ipc: Option<&mut dyn IpcPoller>,
    events: &mut dyn EventPoller,
    clock: &mut dyn RuntimeClock,
    options: &RunOptions,
) -> Result<RunReport, AppError> {
    let mut report = RunReport::default();
    let start_ms = clock.now_ms();
    let deadline = options
        .run_seconds
        .map(|seconds| start_ms.saturating_add(seconds as i64 * 1_000));
    let retry_ms = options.retry_ms.max(5_000);
    let retry_i64 = retry_ms as i64;
    let mut device: Option<DeviceSession<dyn DeviceAdapter>> = None;
    // 上一次成功下发到设备的帧；重连/拔出后清空，下一拍回全帧。
    let mut last_sent: Option<Frame> = None;
    // 上一次发整帧的时间；用于周期性校准，防止设备丢帧后差异永久不再重发。
    let mut last_full_ms = start_ms;
    let mut next_retry_ms = start_ms;
    let mut last_status: Option<bool> = None;
    let mut last_status_ms = start_ms;
    let mut last_stats_ms = start_ms;
    let mut tick_ms = options.tick_ms;

    while !clock.should_stop() {
        let now = clock.now_ms();
        if deadline.is_some_and(|deadline| now >= deadline) {
            break;
        }
        report.ticks += 1;

        // 0) 亮度 / 刷新间隔热更新：改完下一拍生效。
        //    不重启进程、不重新初始化设备——重启会让屏幕重新上电初始化并闪一下。
        if let Some(new_tick) = app.reload_display_options() {
            if new_tick != tick_ms {
                crate::logging::info(&format!("热更新：刷新间隔 {new_tick}ms"));
            }
            tick_ms = new_tick;
        }

        // 1) 设备事件：Removal 关闭 Session；Arrival 立即尝试重连。
        for _ in 0..options.max_events_per_tick {
            match events.try_recv() {
                Some(DeviceEvent::Removal) => {
                    if let Some(mut session) = device.take() {
                        session.close();
                        crate::logging::warn("设备已移除，关闭 Session");
                    }
                    last_sent = None;
                    next_retry_ms = now.saturating_add(retry_i64);
                }
                Some(DeviceEvent::Arrival) => {
                    if device.is_none() {
                        next_retry_ms = now;
                    }
                }
                Some(DeviceEvent::Other) | None => break,
            }
        }

        // 2) 设备重连（有退避，不做重试风暴）。
        if device.is_none() && now >= next_retry_ms {
            report.connect_attempts += 1;
            match try_connect(provider) {
                Ok(Some(mut session)) => {
                    report.connect_successes += 1;
                    // 清屏同步帧：重启（主题/方向/卡片切换）或重连时，串口驱动队列
                    // 里可能残留上一个进程未写完的半帧字节。先发一帧纯黑整帧，
                    // 让设备按帧头重新对齐字节流，再进入正常差分刷新。
                    match clear_frame(&options.physical) {
                        Some(frame) => match session.send_frame(&frame) {
                            Ok(()) => {
                                report.sync_frames += 1;
                                // 留出设备刷新清屏帧的时间：否则首帧内容与清屏帧
                                // 尾部背靠背下发，设备丢掉最后一分块 → 底部残留黑块。
                                if options.sync_settle_ms > 0 {
                                    clock.sleep_ms(options.sync_settle_ms);
                                }
                                device = Some(session);
                                crate::logging::info("设备已连接并初始化（仅一次），已清屏");
                            }
                            Err(err) => {
                                report.send_failures += 1;
                                crate::logging::warn(&format!("清屏同步帧发送失败：{err}"));
                                session.close();
                                next_retry_ms = now.saturating_add(retry_i64);
                            }
                        },
                        None => {
                            device = Some(session);
                            crate::logging::info("设备已连接并初始化（仅一次）");
                        }
                    }
                }
                Ok(None) => {
                    crate::logging::warn(&format!("未发现设备，{}s 后重试", retry_ms / 1_000));
                    next_retry_ms = now.saturating_add(retry_i64);
                }
                Err(err) => {
                    crate::logging::warn(&format!(
                        "设备不可用：{err}，{}s 后重试",
                        retry_ms / 1_000
                    ));
                    next_retry_ms = now.saturating_add(retry_i64);
                }
            }
        }

        // 3) IPC：处理消息；断开时清除旧快照。
        if let Some(ipc) = ipc.as_deref_mut() {
            for _ in 0..options.max_ipc_messages_per_tick {
                let before = ipc.state();
                match ipc.poll_message() {
                    Ok(Some(message)) => match app.handle_ipc_message(&message, now) {
                        Ok(_) => report.ipc_messages += 1,
                        Err(err) => {
                            report.ipc_rejects += 1;
                            crate::logging::warn(&format!("IPC 消息被拒绝：{err}"));
                        }
                    },
                    Ok(None) => {
                        if before == ConnectionState::Connected
                            && ipc.state() == ConnectionState::WaitingClient
                        {
                            app.handle_client_disconnected();
                            crate::logging::info("IPC 客户端断开，已清除旧快照");
                        }
                        break;
                    }
                    Err(err) => {
                        report.ipc_rejects += 1;
                        crate::logging::error(&format!("IPC 读取失败：{err}"));
                        break;
                    }
                }
            }
        }

        // 4) 数据状态 → 渲染真实指标或占位画面。
        let data_state = app.tick(now);
        let frame_result = match data_state {
            DataState::Ready => app.render_with_data(&options.physical),
            DataState::Stale => {
                report.stale_frames += 1;
                app.render_placeholder(&options.physical, "数据过期")
            }
            DataState::WaitingClient => {
                report.waiting_frames += 1;
                app.render_placeholder(&options.physical, "等待客户端")
            }
            DataState::VersionMismatch => {
                report.waiting_frames += 1;
                app.render_placeholder(&options.physical, "版本不兼容")
            }
        };
        let frame = match frame_result {
            Ok(frame) => Some(frame),
            Err(err) => {
                report.render_failures += 1;
                crate::logging::error(&format!("渲染失败：{err}"));
                None
            }
        };

        // 5) 差分下发：内容不变则不发；小范围变化只发脏矩形；否则发完整帧。
        //    设备无双缓冲，整帧重画就是现场看到的闪屏。
        if let (Some(frame), Some(session)) = (frame.as_ref(), device.as_mut()) {
            if session.is_ready() {
                // 差分刷新依赖「发送成功 = 设备显示正确」；按固定间隔强制一次整帧，
                // 把设备内容与本地缓存重新对齐（设备丢帧后差异否则永不重发）。
                let calibration_due = options.full_refresh_ms > 0
                    && now.saturating_sub(last_full_ms) >= options.full_refresh_ms as i64;
                let plan = plan_frame(last_sent.as_ref(), frame, calibration_due);
                match send_planned(session, frame, plan) {
                    SendOutcome::Full => {
                        report.frames_sent += 1;
                        last_sent = Some(frame.clone());
                        last_full_ms = now;
                    }
                    SendOutcome::Partial => {
                        report.frames_partial += 1;
                        last_sent = Some(frame.clone());
                    }
                    SendOutcome::Skipped => report.frames_skipped += 1,
                    SendOutcome::Failed => report.send_failures += 1,
                }
            }
        }
        if device.as_ref().is_some_and(|session| !session.is_ready()) {
            device = None;
            last_sent = None;
            next_retry_ms = now.saturating_add(retry_i64);
        }

        // 上报设备状态：状态变化立即写，稳定时每 5s 心跳一次。
        let connected = device.as_ref().is_some_and(|s| s.is_ready());
        if last_status != Some(connected) || now.saturating_sub(last_status_ms) >= 5_000 {
            write_device_status(
                options.status_file.as_deref(),
                connected,
                if connected {
                    "connected"
                } else {
                    "disconnected"
                },
            );
            last_status = Some(connected);
            last_status_ms = now;
        }

        // 每分钟一行统计：现场用它核对实际发送节拍（只记计数，不含任何凭据）。
        if now.saturating_sub(last_stats_ms) >= 60_000 {
            crate::logging::info(&format!(
                "stats: ticks={} frames_sent={} frames_partial={} frames_skipped={} send_failures={} render_failures={}",
                report.ticks,
                report.frames_sent,
                report.frames_partial,
                report.frames_skipped,
                report.send_failures,
                report.render_failures
            ));
            last_stats_ms = now;
        }

        // 6) 节拍等待：扣掉本拍发送/渲染已用时间，保证刷新周期等于 tick_ms。
        //    单帧传输约 1.9s，若再从整拍开始睡，实际周期会变成「传输 + 节拍」。
        let after_ms = clock.now_ms();
        clock.sleep_ms(tick_wait_ms(
            tick_ms,
            after_ms.saturating_sub(now).max(0) as u64,
            deadline,
            after_ms,
        ));
    }

    if let Some(mut session) = device.take() {
        session.close();
    }
    write_device_status(options.status_file.as_deref(), false, "stopped");
    if let Some(ipc) = ipc {
        ipc.close();
    }
    println!(
        "run end: ticks={} frames_sent={} send_failures={} render_failures={} ipc_messages={} ipc_rejects={} stale_frames={} waiting_frames={}",
        report.ticks,
        report.frames_sent,
        report.send_failures,
        report.render_failures,
        report.ipc_messages,
        report.ipc_rejects,
        report.stale_frames,
        report.waiting_frames
    );
    Ok(report)
}

/// 本拍下发计划：到校准时间就强制整帧，否则按差分结果。
fn plan_frame(previous: Option<&Frame>, current: &Frame, calibration_due: bool) -> FrameSend {
    if calibration_due {
        FrameSend::Full
    } else {
        plan_send(previous, current)
    }
}

/// 一次下发的实际结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendOutcome {
    Full,
    Partial,
    Skipped,
    Failed,
}

/// 按差分计划下发；适配器不支持子矩形时回退全帧。
fn send_planned<A: DeviceAdapter + ?Sized>(
    session: &mut DeviceSession<A>,
    frame: &Frame,
    plan: FrameSend,
) -> SendOutcome {
    match plan {
        FrameSend::Skip => SendOutcome::Skipped,
        FrameSend::Full => match session.send_frame(frame) {
            Ok(()) => SendOutcome::Full,
            Err(err) => {
                crate::logging::error(&format!("发送失败：{err}"));
                session.close();
                SendOutcome::Failed
            }
        },
        FrameSend::Region(region) => match session.send_frame_region(frame, region) {
            Ok(()) => SendOutcome::Partial,
            // 能力缺失而非通信故障：回退整帧，不把设备判死
            Err(SessionError::Send(AdapterError::Unsupported(_))) => {
                match session.send_frame(frame) {
                    Ok(()) => SendOutcome::Full,
                    Err(err) => {
                        crate::logging::error(&format!("发送失败：{err}"));
                        session.close();
                        SendOutcome::Failed
                    }
                }
            }
            Err(err) => {
                crate::logging::error(&format!("发送失败：{err}"));
                session.close();
                SendOutcome::Failed
            }
        },
    }
}

/// 两帧之间必须保留的静止时间（ms）。
///
/// 单帧（320×480 RGB565 = 307,200 B）实测传输约 1.9s；设备无双缓冲，接收期间整屏
/// 在被逐段写入。若刷新间隔小于传输耗时，节拍会退化成「一帧接一帧」的连续重画，
/// 现场表现就是持续闪屏。留 1s 静止时间可避免这种最坏情况。
const MIN_IDLE_MS: u64 = 1_000;

/// 本拍剩余等待：扣掉发送/渲染已用时间，并受运行截止时间约束。
/// 保证实际刷新周期等于 `tick_ms`，而不是「单帧传输耗时 + tick_ms」。
fn tick_wait_ms(tick_ms: u64, spent_ms: u64, deadline: Option<i64>, now_ms: i64) -> u64 {
    let mut wait = tick_ms.saturating_sub(spent_ms).max(MIN_IDLE_MS);
    if let Some(deadline) = deadline {
        wait = wait.min((deadline - now_ms).max(1) as u64);
    }
    wait
}

/// 清屏同步帧：物理分辨率全黑整帧（RGB565LE 全 0）。连接建立后先发它，
/// 让设备把字节流对齐到帧头，吸收上一个进程可能残留的半帧字节。
fn clear_frame(physical: &ScreenProfile) -> Option<Frame> {
    let len = (physical.width as usize) * (physical.height as usize) * 2;
    Frame::new(
        physical.width,
        physical.height,
        screen_core::PixelFormat::Rgb565Le,
        vec![0u8; len],
        None,
    )
    .ok()
}

fn try_connect(
    provider: &mut dyn DeviceProvider,
) -> Result<Option<DeviceSession<dyn DeviceAdapter>>, String> {
    match provider.discover()? {
        Some(info) => provider.open(&info).map(Some),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// 生产实现（Windows）
// ---------------------------------------------------------------------------

#[cfg(windows)]
#[derive(Default)]
pub struct SystemClock;

#[cfg(windows)]
impl SystemClock {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(windows)]
impl RuntimeClock for SystemClock {
    fn now_ms(&mut self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn sleep_ms(&mut self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    fn should_stop(&self) -> bool {
        console_ctrl::install();
        console_ctrl::stop_requested()
    }
}

/// Ctrl+C / Ctrl+Break / 关闭控制台 → 置停止标志，主循环在下一拍自行清理退出。
/// 处理器全局只注册一次（`Once`），重复 install 幂等。
#[cfg(windows)]
mod console_ctrl {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Once;

    use windows::Win32::Foundation::BOOL;
    use windows::Win32::System::Console::SetConsoleCtrlHandler;

    static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
    static INSTALL: Once = Once::new();

    // CTRL_C_EVENT=0 / CTRL_BREAK_EVENT=1 / CTRL_CLOSE_EVENT=2
    unsafe extern "system" fn on_ctrl(ctrl_type: u32) -> BOOL {
        if ctrl_type > 2 {
            return BOOL(0);
        }
        // 第一次：置标志并拦截默认强杀，主循环下一拍自行清理退出。
        // 第二次（主循环疑似卡死）：交还默认处理，允许系统强杀。
        if STOP_REQUESTED.swap(true, Ordering::SeqCst) {
            BOOL(0)
        } else {
            BOOL(1)
        }
    }

    pub fn install() {
        INSTALL.call_once(|| unsafe {
            let _ = SetConsoleCtrlHandler(Some(on_ctrl), BOOL(1));
        });
    }

    pub fn stop_requested() -> bool {
        STOP_REQUESTED.load(Ordering::SeqCst)
    }

    static PARENT_WATCHED: Once = Once::new();

    /// 监视父进程句柄：父进程被强杀（托盘退出、任务管理器、安装器 taskkill）时，
    /// 本进程能自行退出，不会变成孤儿长期占用串口和 hetao-screen.exe。
    pub fn watch_parent(pid: u32) {
        if pid == 0 {
            return;
        }
        PARENT_WATCHED.call_once(move || {
            std::thread::spawn(move || {
                use windows::Win32::Foundation::CloseHandle;
                use windows::Win32::System::Threading::{
                    OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
                };

                // INFINITE：父进程不退出就一直等；拿不到句柄说明父进程已不在。
                if let Ok(handle) = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
                    let _ = unsafe { WaitForSingleObject(handle, 0xFFFF_FFFF) };
                    let _ = unsafe { CloseHandle(handle) };
                }
                STOP_REQUESTED.store(true, Ordering::SeqCst);
            });
        });
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn stop_flag_defaults_to_false() {
            assert!(!super::stop_requested());
        }

        #[test]
        fn install_is_idempotent_and_does_not_panic() {
            super::install();
            super::install();
        }
    }
}

/// 监视父进程：父进程消失时置停止标志，主循环下一拍清理退出。非 Windows 为空实现。
#[cfg(windows)]
pub fn watch_parent(pid: u32) {
    console_ctrl::watch_parent(pid);
}

#[cfg(not(windows))]
pub fn watch_parent(_pid: u32) {}

#[cfg(windows)]
pub struct TurzxDeviceProvider;

#[cfg(windows)]
impl DeviceProvider for TurzxDeviceProvider {
    fn discover(&mut self) -> Result<Option<DeviceInfo>, String> {
        use screen_adapter_turzx_cdc::{DiscoveryError, TurzxCdcAdapter};
        use screen_transport_cdc::CdcTransport;

        let mut adapter = TurzxCdcAdapter::new(CdcTransport::new());
        match adapter.discover() {
            Ok(info) => Ok(Some(info)),
            Err(DiscoveryError::NotFound) => Ok(None),
            Err(err) => Err(err.to_string()),
        }
    }

    fn open(&mut self, info: &DeviceInfo) -> Result<DeviceSession<dyn DeviceAdapter>, String> {
        use screen_adapter_turzx_cdc::TurzxCdcAdapter;
        use screen_transport_cdc::CdcTransport;

        let adapter: Box<dyn DeviceAdapter> = Box::new(TurzxCdcAdapter::new(CdcTransport::new()));
        DeviceSession::connect(adapter, info).map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
pub struct WindowsIpcPoller {
    server: screen_ipc::WindowsPipeServer,
    verifier: screen_ipc::ImageNameVerifier,
}

#[cfg(windows)]
impl WindowsIpcPoller {
    /// `client_exe` 为对端校验期望的映像文件名（生产默认
    /// [`crate::config::DEFAULT_CLIENT_EXE`]，T5 真机回归时由
    /// `[integration].client_exe` 覆盖为推送模拟器文件名）。
    pub fn new(pipe_name: &str, client_exe: &str) -> Result<Self, String> {
        let server = screen_ipc::WindowsPipeServer::new(pipe_name).map_err(|e| e.to_string())?;
        Ok(Self {
            server,
            verifier: screen_ipc::ImageNameVerifier::new(client_exe),
        })
    }
}

#[cfg(windows)]
impl IpcPoller for WindowsIpcPoller {
    fn poll_message(&mut self) -> Result<Option<Message>, String> {
        self.server.poll(&self.verifier).map_err(|e| e.to_string())
    }

    fn state(&self) -> ConnectionState {
        self.server.state()
    }

    fn close(&mut self) {
        self.server.close();
    }
}

#[cfg(windows)]
pub struct ProductionEvents {
    watcher: Option<screen_transport_cdc::DeviceWatcher>,
}

#[cfg(windows)]
impl ProductionEvents {
    pub fn new() -> Self {
        let watcher = match screen_transport_cdc::DeviceWatcher::register() {
            Ok(watcher) => Some(watcher),
            Err(err) => {
                crate::logging::warn(&format!("设备通知注册失败（仍可轮询发现）：{err}"));
                None
            }
        };
        Self { watcher }
    }
}

#[cfg(windows)]
impl Default for ProductionEvents {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
impl EventPoller for ProductionEvents {
    fn try_recv(&mut self) -> Option<DeviceEvent> {
        self.watcher.as_ref().and_then(|watcher| watcher.try_recv())
    }
}

/// 生产入口：装配真实设备/管道/通知/时钟后运行主循环。
#[cfg(windows)]
pub fn run_production(app: &mut ScreenApp, options: RunOptions) -> Result<RunReport, AppError> {
    let pipe = app.config().integration.pipe.clone();
    let client_exe = app.config().integration.client_exe.clone();
    let mut ipc = match WindowsIpcPoller::new(&pipe, &client_exe) {
        Ok(ipc) => Some(ipc),
        Err(err) => {
            crate::logging::error(&format!("IPC 管道不可用：{err}"));
            None
        }
    };
    let mut events = ProductionEvents::new();
    let mut clock = SystemClock::new();
    let mut provider = TurzxDeviceProvider;
    let ipc_ref: Option<&mut dyn IpcPoller> = match ipc.as_mut() {
        Some(ipc) => Some(ipc),
        None => None,
    };
    run_loop(
        app,
        &mut provider,
        ipc_ref,
        &mut events,
        &mut clock,
        &options,
    )
}

#[cfg(not(windows))]
pub fn run_production(_app: &mut ScreenApp, _options: RunOptions) -> Result<RunReport, AppError> {
    Err(AppError::State(
        "生产主循环仅支持 Windows 10/11".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::test_support::{MockAdapter, MockCounters};
    use screen_adapter_api::{DeviceInfo, Region};
    use screen_core::{Frame, PixelFormat, Snapshot, TransportKind};
    use screen_ipc::MessageKind;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct FakeClock {
        now: i64,
        stop: bool,
        sleeps: Vec<u64>,
    }

    impl FakeClock {
        fn new(now: i64) -> Self {
            Self {
                now,
                stop: false,
                sleeps: Vec::new(),
            }
        }
    }

    impl RuntimeClock for FakeClock {
        fn now_ms(&mut self) -> i64 {
            self.now
        }

        fn sleep_ms(&mut self, ms: u64) {
            self.sleeps.push(ms);
            self.now += ms as i64;
        }

        fn should_stop(&self) -> bool {
            self.stop
        }
    }

    fn info() -> DeviceInfo {
        DeviceInfo {
            key: "mock".into(),
            model_id: "mock".into(),
            stable_id: Some("mock".into()),
            transport: TransportKind::Cdc,
            open_path: Some("mock-path".into()),
        }
    }

    struct MockProvider {
        discoveries: VecDeque<Option<DeviceInfo>>,
        counters: Arc<MockCounters>,
        fail_send: Arc<AtomicBool>,
        opens: u32,
    }

    impl MockProvider {
        fn new(discoveries: Vec<Option<DeviceInfo>>) -> Self {
            Self {
                discoveries: discoveries.into(),
                counters: Arc::new(MockCounters::default()),
                fail_send: Arc::new(AtomicBool::new(false)),
                opens: 0,
            }
        }
    }

    impl DeviceProvider for MockProvider {
        fn discover(&mut self) -> Result<Option<DeviceInfo>, String> {
            Ok(self.discoveries.pop_front().unwrap_or(None))
        }

        fn open(
            &mut self,
            device: &DeviceInfo,
        ) -> Result<DeviceSession<dyn DeviceAdapter>, String> {
            self.opens += 1;
            let mut adapter = MockAdapter::new();
            adapter.counters = self.counters.clone();
            adapter.fail_send = self.fail_send.clone();
            let adapter: Box<dyn DeviceAdapter> = Box::new(adapter);
            DeviceSession::connect(adapter, device).map_err(|e| e.to_string())
        }
    }

    struct MockIpc {
        messages: VecDeque<Message>,
        state: ConnectionState,
        disconnected: bool,
        closed: bool,
    }

    impl MockIpc {
        fn new(messages: Vec<Message>) -> Self {
            Self {
                messages: messages.into(),
                state: ConnectionState::WaitingClient,
                disconnected: false,
                closed: false,
            }
        }

        fn with_disconnect(messages: Vec<Message>) -> Self {
            let mut ipc = Self::new(messages);
            ipc.disconnected = true;
            ipc
        }
    }

    impl IpcPoller for MockIpc {
        fn poll_message(&mut self) -> Result<Option<Message>, String> {
            match self.messages.pop_front() {
                Some(message) => {
                    self.state = ConnectionState::Connected;
                    Ok(Some(message))
                }
                None => {
                    if self.disconnected {
                        self.state = ConnectionState::WaitingClient;
                    }
                    Ok(None)
                }
            }
        }

        fn state(&self) -> ConnectionState {
            self.state
        }

        fn close(&mut self) {
            self.closed = true;
            self.state = ConnectionState::Disconnected;
        }
    }

    struct ScheduledEvents {
        events: VecDeque<(usize, DeviceEvent)>,
        calls: usize,
    }

    impl ScheduledEvents {
        fn new(events: Vec<(usize, DeviceEvent)>) -> Self {
            Self {
                events: events.into(),
                calls: 0,
            }
        }
    }

    impl EventPoller for ScheduledEvents {
        fn try_recv(&mut self) -> Option<DeviceEvent> {
            let call = self.calls;
            self.calls += 1;
            match self.events.front() {
                Some((index, event)) if *index == call => {
                    let event = *event;
                    self.events.pop_front();
                    Some(event)
                }
                _ => None,
            }
        }
    }

    fn snapshot_message(seq: u64, ts_ms: i64) -> Message {
        let snapshot = Snapshot::new(seq, ts_ms);
        Message::with_payload(MessageKind::Snapshot, seq, &snapshot).expect("message")
    }

    fn app() -> ScreenApp {
        let missing = std::env::temp_dir().join("hetao-runtime-test-missing.toml");
        let _ = std::fs::remove_file(&missing);
        ScreenApp::init(Some(&missing)).expect("app")
    }

    fn frame_320x480() -> Frame {
        Frame::new(
            PHYSICAL_WIDTH,
            PHYSICAL_HEIGHT,
            PixelFormat::Rgb565Le,
            vec![0u8; (PHYSICAL_WIDTH * PHYSICAL_HEIGHT * 2) as usize],
            Some(8192),
        )
        .expect("frame")
    }

    fn options(seconds: u64) -> RunOptions {
        RunOptions {
            run_seconds: Some(seconds),
            status_file: None,
            // 测试用虚拟时钟，清屏后的真实等待没有必要
            sync_settle_ms: 0,
            ..Default::default()
        }
    }

    #[test]
    fn no_device_waits_with_backoff_without_spinning() {
        let mut app = app();
        let mut provider = MockProvider::new(Vec::new());
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        let report = run_loop(
            &mut app,
            &mut provider,
            None,
            &mut events,
            &mut clock,
            &options(5),
        )
        .expect("run");
        assert_eq!(report.frames_sent, 0);
        assert_eq!(report.waiting_frames, report.ticks);
        assert!(report.connect_attempts <= 2, "5s 内不应反复重连");
        assert_eq!(provider.opens, 0);
    }

    #[test]
    fn first_frame_is_full_and_repeated_identical_ticks_are_skipped() {
        let mut app = app();
        let mut provider = MockProvider::new(vec![Some(info())]);
        let mut ipc = MockIpc::new(vec![
            snapshot_message(1, 0),
            snapshot_message(2, 0),
            snapshot_message(3, 0),
        ]);
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        let report = run_loop(
            &mut app,
            &mut provider,
            Some(&mut ipc),
            &mut events,
            &mut clock,
            &options(3),
        )
        .expect("run");

        // 同一分钟内渲染出的帧完全相同：首拍整帧，后续拍不重发
        assert_eq!(report.frames_sent + report.frames_skipped, 3);
        assert!(report.frames_skipped >= 1, "内容未变不应重画整帧");
        assert_eq!(report.ipc_messages, 3);
        assert_eq!(report.ipc_rejects, 0);
        assert_eq!(provider.opens, 1, "只应打开一次");
        assert_eq!(provider.counters.open_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.counters.init_calls.load(Ordering::SeqCst), 1);
        // 发送调用 = 差分/整帧下发 + 连接时的清屏同步帧
        assert_eq!(
            provider.counters.send_calls.load(Ordering::SeqCst),
            report.frames_sent as u32 + report.sync_frames as u32
        );
        assert!(ipc.closed);
    }

    #[test]
    fn connect_sends_clear_frame_before_content() {
        let mut app = app();
        let mut provider = MockProvider::new(vec![Some(info())]);
        let mut ipc = MockIpc::new(vec![snapshot_message(1, 0)]);
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        let report = run_loop(
            &mut app,
            &mut provider,
            Some(&mut ipc),
            &mut events,
            &mut clock,
            &options(2),
        )
        .expect("run");

        assert_eq!(report.sync_frames, 1, "每次连接恰好一帧清屏帧");
        let frames = provider.counters.frames.lock().expect("frames lock");
        assert!(frames.len() >= 2, "清屏帧后必须还有内容帧");
        let clear = &frames[0];
        assert_eq!(clear.width, PHYSICAL_WIDTH);
        assert_eq!(clear.height, PHYSICAL_HEIGHT);
        assert_eq!(clear.pixel_format, PixelFormat::Rgb565Le);
        assert!(
            clear.pixels.iter().all(|b| *b == 0),
            "清屏帧必须是全黑整帧，让设备按帧头重新对齐字节流"
        );
        assert!(
            frames[1..].iter().any(|f| f.pixels.iter().any(|b| *b != 0)),
            "清屏帧之后必须下发真实内容帧"
        );
    }

    #[test]
    fn clear_frame_send_failure_drops_session() {
        let mut app = app();
        let mut provider = MockProvider::new(vec![Some(info())]);
        provider.fail_send = Arc::new(AtomicBool::new(true));
        let mut ipc = MockIpc::new(vec![snapshot_message(1, 0)]);
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        let report = run_loop(
            &mut app,
            &mut provider,
            Some(&mut ipc),
            &mut events,
            &mut clock,
            &options(3),
        )
        .expect("run");

        assert_eq!(report.sync_frames, 0);
        assert_eq!(report.send_failures, 1, "清屏帧发送失败计为发送失败");
        assert!(provider.counters.close_calls.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn clear_frame_settles_before_content_frame() {
        let mut app = app();
        let mut provider = MockProvider::new(vec![Some(info())]);
        let mut ipc = MockIpc::new(vec![snapshot_message(1, 0)]);
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        let mut opts = options(2);
        opts.sync_settle_ms = 250;
        let report = run_loop(
            &mut app,
            &mut provider,
            Some(&mut ipc),
            &mut events,
            &mut clock,
            &opts,
        )
        .expect("run");

        assert_eq!(report.sync_frames, 1);
        assert!(
            clock.sleeps.contains(&250),
            "清屏帧后必须留出设备刷新时间，否则底部残留黑块：{:?}",
            clock.sleeps
        );
    }

    #[test]
    fn sync_settle_can_be_disabled() {
        let mut app = app();
        let mut provider = MockProvider::new(vec![Some(info())]);
        let mut ipc = MockIpc::new(vec![snapshot_message(1, 0)]);
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        let mut opts = options(2);
        opts.sync_settle_ms = 0;
        let report = run_loop(
            &mut app,
            &mut provider,
            Some(&mut ipc),
            &mut events,
            &mut clock,
            &opts,
        )
        .expect("run");

        assert_eq!(report.sync_frames, 1);
        assert!(!clock.sleeps.contains(&0), "0 表示不额外等待");
    }

    #[test]
    fn stale_data_renders_placeholder_not_last_values() {
        let mut app = app();
        let mut provider = MockProvider::new(vec![Some(info())]);
        let mut ipc = MockIpc::new(vec![snapshot_message(1, 0)]);
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        let report = run_loop(
            &mut app,
            &mut provider,
            Some(&mut ipc),
            &mut events,
            &mut clock,
            &options(5),
        )
        .expect("run");

        assert!(report.stale_frames >= 1);
        assert_eq!(report.frames_sent + report.frames_skipped, report.ticks);
        assert!(report.frames_skipped >= 1, "重复占位画面不应反复重画");
        assert!(app.snapshot().is_some(), "过期不清空快照，但不再用于渲染");
    }

    #[test]
    fn send_failure_drops_session_and_uses_backoff() {
        let mut app = app();
        let mut provider = MockProvider::new(vec![Some(info()), Some(info())]);
        provider.fail_send = Arc::new(AtomicBool::new(true));
        let mut ipc = MockIpc::new(vec![snapshot_message(1, 0)]);
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        let report = run_loop(
            &mut app,
            &mut provider,
            Some(&mut ipc),
            &mut events,
            &mut clock,
            &options(5),
        )
        .expect("run");

        assert_eq!(report.send_failures, 1);
        assert!(provider.counters.close_calls.load(Ordering::SeqCst) >= 1);
        assert!(provider.opens >= 1);
        assert!(report.connect_attempts <= 2, "退避期间不应重试风暴");
    }

    #[test]
    fn removal_closes_session_and_arrival_reconnects() {
        let mut app = app();
        let mut provider = MockProvider::new(vec![Some(info()), Some(info())]);
        let mut ipc = MockIpc::new(vec![snapshot_message(1, 0)]);
        // 第 0/1 次调用无事件（第 1、2 个节拍）；第 2 次 Removal、第 3 次 Arrival。
        let mut events =
            ScheduledEvents::new(vec![(2, DeviceEvent::Removal), (3, DeviceEvent::Arrival)]);
        let mut clock = FakeClock::new(0);
        let report = run_loop(
            &mut app,
            &mut provider,
            Some(&mut ipc),
            &mut events,
            &mut clock,
            &options(4),
        )
        .expect("run");

        assert_eq!(report.connect_successes, 2, "拔插后应重新连接");
        assert_eq!(provider.opens, 2);
        assert_eq!(provider.counters.open_calls.load(Ordering::SeqCst), 2);
        assert_eq!(provider.counters.init_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn ipc_disconnect_clears_old_snapshot() {
        let mut app = app();
        let mut provider = MockProvider::new(vec![Some(info())]);
        let mut ipc = MockIpc::with_disconnect(vec![snapshot_message(1, 0)]);
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        let report = run_loop(
            &mut app,
            &mut provider,
            Some(&mut ipc),
            &mut events,
            &mut clock,
            &options(3),
        )
        .expect("run");

        assert_eq!(report.ipc_messages, 1);
        assert!(app.snapshot().is_none(), "断线后旧快照必须清除");
        assert_eq!(app.ipc_state(), ConnectionState::WaitingClient);
    }

    #[test]
    fn duplicate_snapshot_is_rejected_and_counted() {
        let mut app = app();
        let mut provider = MockProvider::new(vec![Some(info())]);
        let mut ipc = MockIpc::new(vec![snapshot_message(1, 0), snapshot_message(1, 0)]);
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        let report = run_loop(
            &mut app,
            &mut provider,
            Some(&mut ipc),
            &mut events,
            &mut clock,
            &options(2),
        )
        .expect("run");

        assert_eq!(report.ipc_messages, 1);
        assert_eq!(report.ipc_rejects, 1);
    }

    #[test]
    fn run_seconds_exits_and_releases_session() {
        let mut app = app();
        let mut provider = MockProvider::new(vec![Some(info())]);
        let mut ipc = MockIpc::new(vec![snapshot_message(1, 0)]);
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        let report = run_loop(
            &mut app,
            &mut provider,
            Some(&mut ipc),
            &mut events,
            &mut clock,
            &options(2),
        )
        .expect("run");

        assert_eq!(report.ticks, 2);
        assert!(provider.counters.close_calls.load(Ordering::SeqCst) >= 1);
        assert!(ipc.closed);
    }

    #[test]
    fn stop_signal_breaks_loop() {
        let mut app = app();
        let mut provider = MockProvider::new(Vec::new());
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        clock.stop = true;
        let report = run_loop(
            &mut app,
            &mut provider,
            None,
            &mut events,
            &mut clock,
            &options(10),
        )
        .expect("run");
        assert_eq!(report.ticks, 0);
    }

    #[test]
    fn tick_wait_subtracts_send_time() {
        // 5s 拍长、发送用掉 1.9s → 只再睡 3.1s，实际周期仍是 5s
        assert_eq!(tick_wait_ms(5_000, 1_900, None, 0), 3_100);
        assert_eq!(tick_wait_ms(1_000, 0, None, 0), 1_000);
    }

    #[test]
    fn tick_wait_keeps_idle_gap_when_send_exceeds_tick() {
        // 1s/2s 刷新小于单帧传输耗时（约 1.9s）：不能退化成连续重画，
        // 必须留出 MIN_IDLE_MS 的静止时间，否则屏幕持续闪。
        assert_eq!(tick_wait_ms(1_000, 1_900, None, 0), MIN_IDLE_MS);
        assert_eq!(tick_wait_ms(2_000, 1_900, None, 0), MIN_IDLE_MS);
        assert_eq!(tick_wait_ms(1_900, 1_900, None, 0), MIN_IDLE_MS);
        // 3s 拍长仍有 >1s 的静止时间，不受影响
        assert_eq!(tick_wait_ms(3_000, 1_900, None, 0), 1_100);
    }

    #[test]
    fn tick_wait_respects_run_deadline() {
        assert_eq!(tick_wait_ms(5_000, 0, Some(1_200), 1_000), 200);
        assert_eq!(tick_wait_ms(5_000, 0, Some(1_000), 1_000), 1);
    }

    #[test]
    fn calibration_overrides_diff_only_when_due() {
        let frame = frame_320x480();
        assert_eq!(plan_frame(Some(&frame), &frame, false), FrameSend::Skip);
        assert_eq!(plan_frame(Some(&frame), &frame, true), FrameSend::Full);
        assert_eq!(plan_frame(None, &frame, false), FrameSend::Full);
    }

    #[test]
    fn periodic_calibration_sends_full_frame_even_with_static_content() {
        let mut app = app();
        let mut provider = MockProvider::new(vec![Some(info())]);
        let mut ipc = MockIpc::new(vec![
            snapshot_message(1, 0),
            snapshot_message(2, 0),
            snapshot_message(3, 0),
            snapshot_message(4, 0),
            snapshot_message(5, 0),
            snapshot_message(6, 0),
        ]);
        let mut events = ScheduledEvents::new(Vec::new());
        let mut clock = FakeClock::new(0);
        let mut opts = options(6);
        opts.full_refresh_ms = 2_500;
        let report = run_loop(
            &mut app,
            &mut provider,
            Some(&mut ipc),
            &mut events,
            &mut clock,
            &opts,
        )
        .expect("run");

        // 首帧 + 至少一次校准整帧
        assert!(report.frames_sent >= 2, "校准应再发整帧：{report:?}");
        assert!(
            report.frames_partial + report.frames_skipped >= 1,
            "非校准拍不应全是整帧：{report:?}"
        );
    }

    #[test]
    fn skip_plan_writes_nothing() {
        let adapter = MockAdapter::new();
        let counts = adapter.counters.clone();
        let mut session = DeviceSession::connect(Box::new(adapter), &info()).expect("connect");
        assert_eq!(
            send_planned(&mut session, &frame_320x480(), FrameSend::Skip),
            SendOutcome::Skipped
        );
        assert_eq!(counts.send_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn unsupported_region_falls_back_to_full_frame_without_faulting() {
        let adapter = MockAdapter::new();
        let counts = adapter.counters.clone();
        let mut session = DeviceSession::connect(Box::new(adapter), &info()).expect("connect");
        let outcome = send_planned(
            &mut session,
            &frame_320x480(),
            FrameSend::Region(Region::new(0, 100, 320, 40)),
        );
        assert_eq!(outcome, SendOutcome::Full, "无子矩形能力时必须回退整帧");
        assert_eq!(counts.send_calls.load(Ordering::SeqCst), 1);
        assert!(session.is_ready(), "能力缺失不得把 Session 判为故障");
    }

    #[test]
    fn status_json_carries_connected_flag_and_timestamp() {
        let text = status_json(false, 1_234_567, "disconnected");
        let v: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(v["connected"], false);
        assert_eq!(v["updated_ms"], 1_234_567u64);
        assert_eq!(v["reason"], "disconnected");
    }
}
