//! 生产设备 Session：设备打开/初始化/发送/关闭的唯一持有者。
//!
//! 方向切换只允许重新生成并发送完整物理帧，不允许再次打开或初始化。

use std::fmt;

use screen_adapter_api::{AdapterError, DeviceAdapter, DeviceInfo};
use screen_core::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Disconnected,
    Ready,
    Faulted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    AlreadyOpen,
    NotReady,
    Open(AdapterError),
    Init(AdapterError),
    Send(AdapterError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::AlreadyOpen => write!(f, "设备 Session 已存在"),
            SessionError::NotReady => write!(f, "设备 Session 未就绪"),
            SessionError::Open(e) => write!(f, "设备打开失败：{e}"),
            SessionError::Init(e) => write!(f, "设备初始化失败：{e}"),
            SessionError::Send(e) => write!(f, "设备发送失败：{e}"),
        }
    }
}

impl std::error::Error for SessionError {}

pub struct DeviceSession<A: DeviceAdapter + ?Sized> {
    adapter: Box<A>,
    state: SessionState,
}

impl<A: DeviceAdapter + ?Sized> DeviceSession<A> {
    /// 打开并初始化一次；初始化失败会关闭已打开的句柄。
    pub fn connect(mut adapter: Box<A>, device: &DeviceInfo) -> Result<Self, SessionError> {
        adapter.open(device).map_err(SessionError::Open)?;
        match adapter.init() {
            Ok(_) => Ok(Self {
                adapter,
                state: SessionState::Ready,
            }),
            Err(err) => {
                adapter.close();
                Err(SessionError::Init(err))
            }
        }
    }

    /// 断开后重新连接：先关闭旧句柄，再打开/初始化一次；失败不自动重试。
    pub fn reconnect(&mut self, device: &DeviceInfo) -> Result<(), SessionError> {
        self.close();
        self.adapter.open(device).map_err(SessionError::Open)?;
        match self.adapter.init() {
            Ok(_) => {
                self.state = SessionState::Ready;
                Ok(())
            }
            Err(err) => {
                self.adapter.close();
                self.state = SessionState::Disconnected;
                Err(SessionError::Init(err))
            }
        }
    }

    /// 只发送完整帧；不初始化、不重试。发送失败后将 Session 标记为 Faulted。
    pub fn send_frame(&mut self, frame: &Frame) -> Result<(), SessionError> {
        if self.state != SessionState::Ready {
            return Err(SessionError::NotReady);
        }
        match self.adapter.send_frame(frame) {
            Ok(()) => Ok(()),
            Err(err) => {
                self.state = SessionState::Faulted;
                Err(SessionError::Send(err))
            }
        }
    }

    pub fn close(&mut self) {
        if self.state != SessionState::Disconnected {
            self.adapter.close();
            self.state = SessionState::Disconnected;
        }
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn is_ready(&self) -> bool {
        self.state == SessionState::Ready
    }

    pub fn adapter(&self) -> &A {
        &self.adapter
    }
}

impl<A: DeviceAdapter + ?Sized> Drop for DeviceSession<A> {
    fn drop(&mut self) {
        self.adapter.close();
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    use screen_adapter_api::{AdapterError, DeviceAdapter, DeviceInfo, Health, InitReport};
    use screen_core::{DeviceCapabilities, Frame, Source, TransportKind};

    #[derive(Default)]
    pub struct MockCounters {
        pub open_calls: AtomicU32,
        pub init_calls: AtomicU32,
        pub send_calls: AtomicU32,
        pub close_calls: AtomicU32,
        pub frames: Mutex<Vec<Frame>>,
    }

    pub struct MockAdapter {
        caps: DeviceCapabilities,
        pub counters: Arc<MockCounters>,
        pub fail_open: Arc<AtomicBool>,
        pub fail_init: Arc<AtomicBool>,
        pub fail_send: Arc<AtomicBool>,
        opened: bool,
    }

    impl MockAdapter {
        pub fn new() -> Self {
            Self {
                caps: DeviceCapabilities::registered(
                    "mock",
                    TransportKind::Cdc,
                    false,
                    Source::Unknown,
                ),
                counters: Arc::new(MockCounters::default()),
                fail_open: Arc::new(AtomicBool::new(false)),
                fail_init: Arc::new(AtomicBool::new(false)),
                fail_send: Arc::new(AtomicBool::new(false)),
                opened: false,
            }
        }
    }

    impl Default for MockAdapter {
        fn default() -> Self {
            Self::new()
        }
    }

    impl DeviceAdapter for MockAdapter {
        fn model_id(&self) -> &str {
            "mock"
        }

        fn capabilities(&self) -> &DeviceCapabilities {
            &self.caps
        }

        fn open(&mut self, _device: &DeviceInfo) -> Result<(), AdapterError> {
            if self.fail_open.load(Ordering::SeqCst) {
                return Err(AdapterError::Busy);
            }
            self.counters.open_calls.fetch_add(1, Ordering::SeqCst);
            self.opened = true;
            Ok(())
        }

        fn init(&mut self) -> Result<InitReport, AdapterError> {
            if !self.opened {
                return Err(AdapterError::Disconnected);
            }
            if self.fail_init.load(Ordering::SeqCst) {
                return Err(AdapterError::Io("mock init failure".to_string()));
            }
            self.counters.init_calls.fetch_add(1, Ordering::SeqCst);
            Ok(InitReport::default())
        }

        fn send_frame(&mut self, frame: &Frame) -> Result<(), AdapterError> {
            if !self.opened {
                return Err(AdapterError::Disconnected);
            }
            if self.fail_send.load(Ordering::SeqCst) {
                return Err(AdapterError::Disconnected);
            }
            self.counters.send_calls.fetch_add(1, Ordering::SeqCst);
            self.counters
                .frames
                .lock()
                .expect("frames lock")
                .push(frame.clone());
            Ok(())
        }

        fn set_brightness(&mut self, _level: u8) -> Result<(), AdapterError> {
            Err(AdapterError::Unsupported("mock brightness"))
        }

        fn poll_health(&mut self) -> Result<Health, AdapterError> {
            if !self.opened {
                return Err(AdapterError::Disconnected);
            }
            Ok(Health::ok())
        }

        fn close(&mut self) {
            self.counters.close_calls.fetch_add(1, Ordering::SeqCst);
            self.opened = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{MockAdapter, MockCounters};
    use super::*;
    use screen_adapter_api::DeviceInfo;
    use screen_core::{Frame, PixelFormat, TransportKind};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    fn info() -> DeviceInfo {
        DeviceInfo {
            key: "mock".into(),
            model_id: "mock".into(),
            stable_id: Some("mock".into()),
            transport: TransportKind::Cdc,
            open_path: Some("mock-path".into()),
        }
    }

    fn frame() -> Frame {
        Frame::new(
            320,
            480,
            PixelFormat::Rgb565Le,
            vec![0u8; 320 * 480 * 2],
            Some(8192),
        )
        .expect("frame")
    }

    fn counters(adapter: &MockAdapter) -> Arc<MockCounters> {
        adapter.counters.clone()
    }

    #[test]
    fn connect_opens_and_initializes_once_then_refresh_does_not_init() {
        let adapter = MockAdapter::new();
        let counts = counters(&adapter);
        let mut session = DeviceSession::connect(Box::new(adapter), &info()).expect("connect");
        assert_eq!(session.state(), SessionState::Ready);
        assert_eq!(counts.open_calls.load(Ordering::SeqCst), 1);
        assert_eq!(counts.init_calls.load(Ordering::SeqCst), 1);

        session.send_frame(&frame()).expect("send 1");
        session.send_frame(&frame()).expect("send 2");
        assert_eq!(counts.open_calls.load(Ordering::SeqCst), 1);
        assert_eq!(counts.init_calls.load(Ordering::SeqCst), 1);
        assert_eq!(counts.send_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn send_after_close_is_rejected_without_adapter_call() {
        let adapter = MockAdapter::new();
        let counts = counters(&adapter);
        let mut session = DeviceSession::connect(Box::new(adapter), &info()).expect("connect");
        session.close();
        assert_eq!(session.state(), SessionState::Disconnected);
        assert_eq!(session.send_frame(&frame()), Err(SessionError::NotReady));
        assert_eq!(counts.send_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn send_failure_faults_session_and_blocks_further_sends() {
        let adapter = MockAdapter::new();
        let counts = counters(&adapter);
        adapter.fail_send.store(true, Ordering::SeqCst);
        let mut session = DeviceSession::connect(Box::new(adapter), &info()).expect("connect");
        let err = match session.send_frame(&frame()) {
            Ok(()) => panic!("send should fail"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            SessionError::Send(AdapterError::Disconnected)
        ));
        assert_eq!(session.state(), SessionState::Faulted);
        assert_eq!(session.send_frame(&frame()), Err(SessionError::NotReady));
        assert_eq!(counts.send_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn reconnect_opens_and_initializes_once_after_fault() {
        let adapter = MockAdapter::new();
        let counts = counters(&adapter);
        adapter.fail_send.store(true, Ordering::SeqCst);
        let mut session = DeviceSession::connect(Box::new(adapter), &info()).expect("connect");
        assert!(session.send_frame(&frame()).is_err());
        session.adapter.fail_send.store(false, Ordering::SeqCst);
        session.reconnect(&info()).expect("reconnect");
        assert_eq!(session.state(), SessionState::Ready);
        assert_eq!(counts.open_calls.load(Ordering::SeqCst), 2);
        assert_eq!(counts.init_calls.load(Ordering::SeqCst), 2);
        session.send_frame(&frame()).expect("send after reconnect");
    }

    #[test]
    fn open_failure_is_reported_without_init() {
        let adapter = MockAdapter::new();
        let counts = counters(&adapter);
        adapter.fail_open.store(true, Ordering::SeqCst);
        let err = match DeviceSession::connect(Box::new(adapter), &info()) {
            Ok(_) => panic!("open should fail"),
            Err(err) => err,
        };
        assert!(matches!(err, SessionError::Open(AdapterError::Busy)));
        assert_eq!(counts.open_calls.load(Ordering::SeqCst), 0);
        assert_eq!(counts.init_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn init_failure_closes_handle_and_does_not_create_ready_session() {
        let adapter = MockAdapter::new();
        let counts = counters(&adapter);
        adapter.fail_init.store(true, Ordering::SeqCst);
        let err = match DeviceSession::connect(Box::new(adapter), &info()) {
            Ok(_) => panic!("init should fail"),
            Err(err) => err,
        };
        assert!(matches!(err, SessionError::Init(_)));
        assert_eq!(counts.open_calls.load(Ordering::SeqCst), 1);
        assert_eq!(counts.close_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn drop_closes_open_session() {
        let adapter = MockAdapter::new();
        let counts = counters(&adapter);
        {
            let _session = DeviceSession::connect(Box::new(adapter), &info()).expect("connect");
        }
        assert!(counts.close_calls.load(Ordering::SeqCst) >= 1);
    }
}
