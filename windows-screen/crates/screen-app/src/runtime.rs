//! 生产主循环：设备发现/连接/重连、IPC 快照接收、渲染发送、占位画面、优雅退出。
//!
//! 外部依赖通过 trait 注入，便于用 mock 做状态转移测试；生产实现位于本文件底部。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use screen_adapter_api::{DeviceAdapter, DeviceInfo};
use screen_core::{Orientation, ScreenProfile};
use screen_data::DataState;
use screen_ipc::{ConnectionState, Message};
use screen_transport_cdc::DeviceEvent;

use crate::app::{AppError, ScreenApp};
use crate::session::DeviceSession;

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
        }
    }
}

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
    let mut next_retry_ms = start_ms;

    while !clock.should_stop() {
        let now = clock.now_ms();
        if deadline.is_some_and(|deadline| now >= deadline) {
            break;
        }
        report.ticks += 1;

        // 1) 设备事件：Removal 关闭 Session；Arrival 立即尝试重连。
        for _ in 0..options.max_events_per_tick {
            match events.try_recv() {
                Some(DeviceEvent::Removal) => {
                    if let Some(mut session) = device.take() {
                        session.close();
                        eprintln!("设备已移除，关闭 Session");
                    }
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
                Ok(Some(session)) => {
                    report.connect_successes += 1;
                    device = Some(session);
                    eprintln!("设备已连接并初始化（仅一次）");
                }
                Ok(None) => {
                    eprintln!("未发现设备，{}s 后重试", retry_ms / 1_000);
                    next_retry_ms = now.saturating_add(retry_i64);
                }
                Err(err) => {
                    eprintln!("设备不可用：{err}，{}s 后重试", retry_ms / 1_000);
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
                            eprintln!("IPC 消息被拒绝：{err}");
                        }
                    },
                    Ok(None) => {
                        if before == ConnectionState::Connected
                            && ipc.state() == ConnectionState::WaitingClient
                        {
                            app.handle_client_disconnected();
                            eprintln!("IPC 客户端断开，已清除旧快照");
                        }
                        break;
                    }
                    Err(err) => {
                        report.ipc_rejects += 1;
                        eprintln!("IPC 读取失败：{err}");
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
                eprintln!("渲染失败：{err}");
                None
            }
        };

        // 5) 发送完整帧；失败则关闭 Session 并进入退避重连。
        if let (Some(frame), Some(session)) = (frame.as_ref(), device.as_mut()) {
            if session.is_ready() {
                match session.send_frame(frame) {
                    Ok(()) => report.frames_sent += 1,
                    Err(err) => {
                        report.send_failures += 1;
                        eprintln!("发送失败：{err}");
                        session.close();
                    }
                }
            }
        }
        if device.as_ref().is_some_and(|session| !session.is_ready()) {
            device = None;
            next_retry_ms = now.saturating_add(retry_i64);
        }

        // 6) 节拍等待（不空转）。
        let wait = match deadline {
            Some(deadline) => options.tick_ms.min((deadline - now).max(1) as u64),
            None => options.tick_ms,
        };
        clock.sleep_ms(wait.max(1));
    }

    if let Some(mut session) = device.take() {
        session.close();
    }
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
                eprintln!("设备通知注册失败（仍可轮询发现）：{err}");
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
            eprintln!("IPC 管道不可用：{err}");
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
    use screen_adapter_api::DeviceInfo;
    use screen_core::{Snapshot, TransportKind};
    use screen_ipc::MessageKind;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct FakeClock {
        now: i64,
        stop: bool,
    }

    impl FakeClock {
        fn new(now: i64) -> Self {
            Self { now, stop: false }
        }
    }

    impl RuntimeClock for FakeClock {
        fn now_ms(&mut self) -> i64 {
            self.now
        }

        fn sleep_ms(&mut self, ms: u64) {
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

    fn options(seconds: u64) -> RunOptions {
        RunOptions {
            run_seconds: Some(seconds),
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
    fn ready_snapshots_are_rendered_and_sent_each_tick() {
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

        assert_eq!(report.frames_sent, 3);
        assert_eq!(report.ipc_messages, 3);
        assert_eq!(report.ipc_rejects, 0);
        assert_eq!(provider.opens, 1, "只应打开一次");
        assert_eq!(provider.counters.open_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.counters.init_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.counters.send_calls.load(Ordering::SeqCst), 3);
        assert!(ipc.closed);
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
        assert_eq!(report.frames_sent, 5);
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
}
