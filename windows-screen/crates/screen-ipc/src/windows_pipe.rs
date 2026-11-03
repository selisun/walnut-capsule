//! Windows 命名管道服务端（仅入站）。单实例、仅当前用户 DACL、拒绝远程客户端。
//!
//! 采用 `PIPE_NOWAIT` 轮询，避免阻塞主循环；每条消息按 `screen-ipc` 的 22 字节头 + payload 解析。

#![cfg(windows)]

use std::mem::size_of;

use windows::core::{HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, BOOL, ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_CONNECTED,
    ERROR_PIPE_LISTENING, ERROR_PIPE_NOT_CONNECTED, HANDLE, HLOCAL,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    ReadFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    PeekNamedPipe, NAMED_PIPE_MODE, PIPE_NOWAIT, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::error::IpcError;
use crate::header::{Header, HEADER_LEN, MAX_FRAME_BYTES};
use crate::message::Message;
use crate::transport::{ConnectionState, PeerInfo, PeerVerifier};

pub const DEFAULT_PIPE_NAME: &str = "hetao-screen";
/// 仅对象所有者（当前用户）可访问；不授予其他用户/匿名/远程。
const SDDL_CURRENT_USER: &str = "D:(A;;GA;;;OW)";
const IN_BUFFER_BYTES: u32 = 64 * 1024;

/// 配置值 → 完整管道路径。空值使用默认名；已有 `\\.\pipe\` 前缀则原样使用。
pub fn normalize_pipe_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return format!(r"\\.\pipe\{DEFAULT_PIPE_NAME}");
    }
    if let Some(rest) = trimmed.strip_prefix(r"\\.\pipe\") {
        return format!(r"\\.\pipe\{rest}");
    }
    if let Some(rest) = trimmed.strip_prefix(r"\pipe\") {
        return format!(r"\\.\pipe\{rest}");
    }
    format!(r"\\.\pipe\{trimmed}")
}

pub struct WindowsPipeServer {
    full_name: String,
    handle: Option<HANDLE>,
    connected: bool,
    pending: Vec<u8>,
    security: Option<PSECURITY_DESCRIPTOR>,
    peer: Option<PeerInfo>,
}

impl WindowsPipeServer {
    /// 创建单实例服务端。`pipe_name` 可为空（默认 `hetao-screen`）。
    pub fn new(pipe_name: &str) -> Result<Self, IpcError> {
        let full_name = normalize_pipe_name(pipe_name);
        let wide: Vec<u16> = full_name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut security = PSECURITY_DESCRIPTOR(std::ptr::null_mut());
        let sddl: Vec<u16> = SDDL_CURRENT_USER
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR::from_raw(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut security,
                None,
            )
        }
        .map_err(|e| IpcError::Platform(format!("SDDL 安全描述符创建失败：{e}")))?;

        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security.0,
            bInheritHandle: BOOL(0),
        };
        let open_mode =
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_INBOUND.0 | FILE_FLAG_FIRST_PIPE_INSTANCE.0);
        let pipe_mode = NAMED_PIPE_MODE(
            PIPE_TYPE_BYTE.0 | PIPE_READMODE_BYTE.0 | PIPE_NOWAIT.0 | PIPE_REJECT_REMOTE_CLIENTS.0,
        );
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR::from_raw(wide.as_ptr()),
                open_mode,
                pipe_mode,
                1,
                0,
                IN_BUFFER_BYTES,
                0,
                Some(&attributes),
            )
        };
        if handle.is_invalid() {
            unsafe {
                let _ = LocalFree(HLOCAL(security.0));
            }
            return Err(IpcError::Platform(format!(
                "CreateNamedPipeW 失败：{full_name}"
            )));
        }
        Ok(Self {
            full_name,
            handle: Some(handle),
            connected: false,
            pending: Vec::new(),
            security: Some(security),
            peer: None,
        })
    }

    pub fn pipe_path(&self) -> &str {
        &self.full_name
    }

    pub fn state(&self) -> ConnectionState {
        if self.handle.is_none() {
            ConnectionState::Disconnected
        } else if self.connected {
            ConnectionState::Connected
        } else {
            ConnectionState::WaitingClient
        }
    }

    pub fn peer(&self) -> Option<&PeerInfo> {
        self.peer.as_ref()
    }

    /// 非阻塞轮询：连接 → 身份校验 → 读取/组帧。未就绪返回 `Ok(None)`。
    pub fn poll(&mut self, verifier: &dyn PeerVerifier) -> Result<Option<Message>, IpcError> {
        let Some(handle) = self.handle else {
            return Ok(None);
        };
        if !self.connected {
            match unsafe { ConnectNamedPipe(handle, None) } {
                Ok(()) => {}
                Err(e) if e.code() == HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) => {}
                Err(e) if e.code() == HRESULT::from_win32(ERROR_PIPE_LISTENING.0) => {
                    return Ok(None);
                }
                Err(e) => {
                    return Err(IpcError::Platform(format!("ConnectNamedPipe：{e}")));
                }
            }
            self.connected = true;
            let peer = query_peer(handle)?;
            if let Err(err) = verifier.verify(&peer) {
                self.disconnect();
                return Err(err);
            }
            self.peer = Some(peer);
        }

        // 先消费上一轮已读入 `pending` 的完整消息。
        // 关键：不能只依赖 PeekNamedPipe 有新字节——一次 ReadFile 会把 OS 缓冲全部搬进
        // `pending`，此时 OS 缓冲为空但 `pending` 里可能还有多条完整消息。
        if let Some(message) = self.take_pending_message()? {
            return Ok(Some(message));
        }

        let mut available = 0u32;
        match unsafe { PeekNamedPipe(handle, None, 0, None, Some(&mut available), None) } {
            Ok(()) => {}
            Err(e) if is_disconnect(&e) => {
                self.disconnect();
                return Ok(None);
            }
            Err(e) => return Err(IpcError::Platform(format!("PeekNamedPipe：{e}"))),
        }
        if available == 0 {
            return Ok(None);
        }
        let total_available = self.pending.len().saturating_add(available as usize);
        if total_available > MAX_FRAME_BYTES {
            self.pending.clear();
            return Err(IpcError::PayloadTooLarge {
                max: MAX_FRAME_BYTES,
                got: total_available,
            });
        }
        let mut buffer = vec![0u8; available as usize];
        let mut read = 0u32;
        match unsafe { ReadFile(handle, Some(&mut buffer), Some(&mut read), None) } {
            Ok(()) => {}
            Err(e) if e.code() == HRESULT::from_win32(ERROR_NO_DATA.0) => return Ok(None),
            Err(e) if is_disconnect(&e) => {
                self.disconnect();
                return Ok(None);
            }
            Err(e) => return Err(IpcError::Platform(format!("ReadFile：{e}"))),
        }
        if read == 0 {
            return Ok(None);
        }
        buffer.truncate(read as usize);
        self.pending.extend_from_slice(&buffer);

        self.take_pending_message()
    }

    /// 从 `pending` 取出恰好一条完整消息；不足一条返回 `None`。
    fn take_pending_message(&mut self) -> Result<Option<Message>, IpcError> {
        if self.pending.len() < HEADER_LEN {
            return Ok(None);
        }
        let header = match Header::decode(&self.pending[..HEADER_LEN]) {
            Ok(header) => header,
            Err(err) => {
                self.pending.clear();
                return Err(err);
            }
        };
        let total = HEADER_LEN + header.payload_len as usize;
        if self.pending.len() < total {
            return Ok(None);
        }
        let bytes: Vec<u8> = self.pending.drain(..total).collect();
        Message::from_bytes(&bytes).map(Some)
    }

    /// 断开当前客户端但保留服务端（等待下一个客户端）。
    pub fn disconnect(&mut self) {
        if let Some(handle) = self.handle {
            unsafe {
                let _ = DisconnectNamedPipe(handle);
            }
        }
        self.connected = false;
        self.peer = None;
        self.pending.clear();
    }

    /// 关闭服务端并释放句柄/安全描述符。
    pub fn close(&mut self) {
        if let Some(handle) = self.handle.take() {
            if self.connected {
                unsafe {
                    let _ = DisconnectNamedPipe(handle);
                }
            }
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
        if let Some(security) = self.security.take() {
            unsafe {
                let _ = LocalFree(HLOCAL(security.0));
            }
        }
        self.connected = false;
        self.peer = None;
        self.pending.clear();
    }
}

impl Drop for WindowsPipeServer {
    fn drop(&mut self) {
        self.close();
    }
}

fn is_disconnect(error: &windows::core::Error) -> bool {
    let code = error.code();
    code == HRESULT::from_win32(ERROR_BROKEN_PIPE.0)
        || code == HRESULT::from_win32(ERROR_PIPE_NOT_CONNECTED.0)
}

fn query_peer(handle: HANDLE) -> Result<PeerInfo, IpcError> {
    let mut process_id = 0u32;
    unsafe { GetNamedPipeClientProcessId(handle, &mut process_id) }
        .map_err(|e| IpcError::Platform(format!("GetNamedPipeClientProcessId：{e}")))?;
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
        .map_err(|e| IpcError::Platform(format!("OpenProcess：{e}")))?;
    let mut buffer = vec![0u16; 1024];
    let mut len = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR::from_raw(buffer.as_mut_ptr()),
            &mut len,
        )
    };
    unsafe {
        let _ = CloseHandle(process);
    }
    result.map_err(|e| IpcError::Platform(format!("QueryFullProcessImageNameW：{e}")))?;
    let image_path = String::from_utf16_lossy(&buffer[..len as usize]);
    Ok(PeerInfo {
        process_id,
        user_sid: None,
        image_path: Some(image_path),
        image_sha256: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::MessageKind;
    use crate::peer::ImageNameVerifier;
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    static NEXT_NAME: AtomicU32 = AtomicU32::new(0);

    fn unique_pipe() -> String {
        format!(
            "hetao-test-{}-{}",
            std::process::id(),
            NEXT_NAME.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn current_exe_name() -> String {
        std::env::current_exe()
            .expect("current_exe")
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .to_string()
    }

    fn sample_bytes(seq: u64) -> Vec<u8> {
        let message =
            Message::with_payload(MessageKind::Snapshot, seq, &vec![1u8, 2, 3]).expect("message");
        message.to_bytes().expect("bytes")
    }

    fn spawn_client(pipe_path: String, bytes: Vec<u8>, done: Arc<AtomicBool>) {
        std::thread::spawn(move || {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(&pipe_path)
                .expect("open pipe");
            file.write_all(&bytes).expect("write message");
            file.flush().expect("flush");
            let deadline = Instant::now() + Duration::from_secs(2);
            while !done.load(Ordering::Relaxed) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        });
    }

    #[test]
    fn pipe_name_normalization() {
        assert_eq!(normalize_pipe_name(""), r"\\.\pipe\hetao-screen");
        assert_eq!(normalize_pipe_name("  "), r"\\.\pipe\hetao-screen");
        assert_eq!(normalize_pipe_name("custom"), r"\\.\pipe\custom");
        assert_eq!(normalize_pipe_name(r"\\.\pipe\custom"), r"\\.\pipe\custom");
        assert_eq!(normalize_pipe_name(r"\pipe\custom"), r"\\.\pipe\custom");
    }

    #[test]
    fn server_creates_and_releases_pipe() {
        let name = unique_pipe();
        let mut server = WindowsPipeServer::new(&name).expect("server");
        assert_eq!(server.state(), ConnectionState::WaitingClient);
        assert_eq!(server.pipe_path(), format!(r"\\.\pipe\{name}"));
        assert!(server.poll(&ImageNameVerifier::new("nobody.exe")).is_ok());
        server.close();
        assert_eq!(server.state(), ConnectionState::Disconnected);
        // 句柄释放后同名管道可再次创建
        let server2 = WindowsPipeServer::new(&name).expect("recreate");
        assert_eq!(server2.state(), ConnectionState::WaitingClient);
    }

    #[test]
    fn receives_message_from_test_client() {
        let name = unique_pipe();
        let mut server = WindowsPipeServer::new(&name).expect("server");
        let pipe_path = server.pipe_path().to_string();
        let verifier = ImageNameVerifier::new(current_exe_name());
        let done = Arc::new(AtomicBool::new(false));
        spawn_client(pipe_path, sample_bytes(7), done.clone());

        let deadline = Instant::now() + Duration::from_secs(3);
        let message = loop {
            match server.poll(&verifier) {
                Ok(Some(message)) => break message,
                Ok(None) => {}
                Err(err) => panic!("poll error: {err}"),
            }
            assert!(
                Instant::now() < deadline,
                "等待客户端消息超时（state={:?}）",
                server.state()
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        done.store(true, Ordering::Relaxed);
        assert_eq!(message.header.seq, 7);
        assert_eq!(message.header.kind, MessageKind::Snapshot);
        assert_eq!(server.state(), ConnectionState::Connected);
        assert!(server.peer().is_some());
    }

    #[test]
    fn wrong_image_name_is_refused() {
        let name = unique_pipe();
        let mut server = WindowsPipeServer::new(&name).expect("server");
        let pipe_path = server.pipe_path().to_string();
        let verifier = ImageNameVerifier::new("definitely-not-this-process.exe");
        let done = Arc::new(AtomicBool::new(false));
        spawn_client(pipe_path, sample_bytes(1), done.clone());

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match server.poll(&verifier) {
                Ok(Some(_)) => panic!("不应接受错误映像"),
                Ok(None) => {}
                Err(IpcError::Refused(_)) => break,
                Err(err) => panic!("意外错误：{err}"),
            }
            assert!(Instant::now() < deadline, "等待拒绝超时");
            std::thread::sleep(Duration::from_millis(5));
        }
        done.store(true, Ordering::Relaxed);
        assert_eq!(server.state(), ConnectionState::WaitingClient);
    }

    #[test]
    fn fragmented_writes_are_reassembled() {
        let name = unique_pipe();
        let mut server = WindowsPipeServer::new(&name).expect("server");
        let pipe_path = server.pipe_path().to_string();
        let verifier = ImageNameVerifier::new(current_exe_name());
        let bytes = sample_bytes(9);
        let mid = bytes.len() / 2;
        let parts = [bytes[..mid].to_vec(), bytes[mid..].to_vec()];
        let done = Arc::new(AtomicBool::new(false));
        let done_in_thread = done.clone();
        std::thread::spawn(move || {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(&pipe_path)
                .expect("open pipe");
            for part in &parts {
                file.write_all(part).expect("write part");
                file.flush().expect("flush");
                std::thread::sleep(Duration::from_millis(30));
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while !done_in_thread.load(Ordering::Relaxed) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        });

        let deadline = Instant::now() + Duration::from_secs(3);
        let message = loop {
            match server.poll(&verifier) {
                Ok(Some(message)) => break message,
                Ok(None) => {}
                Err(err) => panic!("poll error: {err}"),
            }
            assert!(Instant::now() < deadline, "等待分片消息超时");
            std::thread::sleep(Duration::from_millis(5));
        };
        done.store(true, Ordering::Relaxed);
        assert_eq!(message.header.seq, 9);
        assert_eq!(message.header.kind, MessageKind::Snapshot);
        assert_eq!(server.state(), ConnectionState::Connected);
    }

    #[test]
    fn reconnect_after_disconnect_accepts_new_client() {
        let name = unique_pipe();
        let mut server = WindowsPipeServer::new(&name).expect("server");
        let pipe_path = server.pipe_path().to_string();
        let verifier = ImageNameVerifier::new(current_exe_name());

        // 第一个客户端：连上、发一条、断开
        let done = Arc::new(AtomicBool::new(false));
        spawn_client(pipe_path.clone(), sample_bytes(1), done.clone());
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match server.poll(&verifier) {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(err) => panic!("poll error: {err}"),
            }
            assert!(Instant::now() < deadline, "首客户端超时");
            std::thread::sleep(Duration::from_millis(5));
        }
        done.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(2);
        while server.state() == ConnectionState::Connected {
            assert!(Instant::now() < deadline, "等待断开超时");
            let _ = server.poll(&verifier);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(server.state(), ConnectionState::WaitingClient);

        // 第二个客户端：同一服务端实例继续接受新连接（T5 R8 反复启停对应路径）
        let done = Arc::new(AtomicBool::new(false));
        spawn_client(pipe_path, sample_bytes(2), done.clone());
        let deadline = Instant::now() + Duration::from_secs(3);
        let message = loop {
            match server.poll(&verifier) {
                Ok(Some(message)) => break message,
                Ok(None) => {}
                Err(err) => panic!("poll error: {err}"),
            }
            assert!(Instant::now() < deadline, "第二客户端超时");
            std::thread::sleep(Duration::from_millis(5));
        };
        done.store(true, Ordering::Relaxed);
        assert_eq!(message.header.seq, 2);
        assert_eq!(server.state(), ConnectionState::Connected);
    }

    #[test]
    fn snapshot_json_roundtrip_over_real_pipe() {
        let name = unique_pipe();
        let mut server = WindowsPipeServer::new(&name).expect("server");
        let pipe_path = server.pipe_path().to_string();
        let verifier = ImageNameVerifier::new(current_exe_name());

        let mut snapshot = screen_core::Snapshot::new(42, 1_700_000_000_000);
        snapshot.metrics.cpu = Some(screen_core::CpuMetrics {
            usage_pct: Some(55.5),
            temp_c: Some(48.0),
            ..Default::default()
        });
        snapshot.metrics.memory = Some(screen_core::MemoryMetrics {
            usage_pct: Some(61.0),
            used_bytes: Some(8 * 1024 * 1024 * 1024),
            total_bytes: Some(16 * 1024 * 1024 * 1024),
            ..Default::default()
        });
        let message = Message::with_payload(MessageKind::Snapshot, 42, &snapshot).expect("message");
        let bytes = message.to_bytes().expect("bytes");

        let done = Arc::new(AtomicBool::new(false));
        spawn_client(pipe_path, bytes, done.clone());
        let deadline = Instant::now() + Duration::from_secs(3);
        let received = loop {
            match server.poll(&verifier) {
                Ok(Some(message)) => break message,
                Ok(None) => {}
                Err(err) => panic!("poll error: {err}"),
            }
            assert!(Instant::now() < deadline, "等待快照超时");
            std::thread::sleep(Duration::from_millis(5));
        };
        done.store(true, Ordering::Relaxed);
        let decoded: screen_core::Snapshot = received.decode_payload().expect("decode");
        assert_eq!(decoded.seq, 42);
        assert_eq!(decoded.metrics.cpu.and_then(|c| c.usage_pct), Some(55.5));
        assert_eq!(
            decoded.metrics.memory.and_then(|m| m.total_bytes),
            Some(16 * 1024 * 1024 * 1024)
        );
    }

    #[test]
    fn buffered_messages_survive_empty_pipe_buffer() {
        // 回归：一次 ReadFile 会把 OS 缓冲整批搬进 pending，但只返回一条消息。
        // 若后续 poll 只看 PeekNamedPipe（此时 OS 缓冲已空）就会返回 None，
        // 调用方据此 break → 每拍只消费一条，积压导致数据长期过期（T5 实测 stale_frames=59/62）。
        let name = unique_pipe();
        let mut server = WindowsPipeServer::new(&name).expect("server");
        let pipe_path = server.pipe_path().to_string();
        let verifier = ImageNameVerifier::new(current_exe_name());

        let mut batch = Vec::new();
        for seq in 1..=3u64 {
            batch.extend_from_slice(&sample_bytes(seq));
        }
        let done = Arc::new(AtomicBool::new(false));
        spawn_client(pipe_path, batch, done.clone());

        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while seen.len() < 3 {
            match server.poll(&verifier) {
                Ok(Some(message)) => seen.push(message.header.seq),
                Ok(None) => {}
                Err(err) => panic!("poll error: {err}"),
            }
            assert!(
                Instant::now() < deadline,
                "等待 3 条缓冲消息超时，已收到 {seen:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        done.store(true, Ordering::Relaxed);
        assert_eq!(seen, vec![1, 2, 3], "pending 缓冲必须按序全部吐出");
        assert!(server.poll(&verifier).expect("poll").is_none());
    }

    #[test]
    fn client_disconnect_returns_to_waiting() {
        let name = unique_pipe();
        let mut server = WindowsPipeServer::new(&name).expect("server");
        let pipe_path = server.pipe_path().to_string();
        let verifier = ImageNameVerifier::new(current_exe_name());
        let done = Arc::new(AtomicBool::new(false));
        spawn_client(pipe_path, sample_bytes(3), done.clone());

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match server.poll(&verifier) {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(err) => panic!("poll error: {err}"),
            }
            assert!(Instant::now() < deadline, "等待消息超时");
            std::thread::sleep(Duration::from_millis(5));
        }
        done.store(true, Ordering::Relaxed);

        let deadline = Instant::now() + Duration::from_secs(2);
        while server.state() == ConnectionState::Connected {
            assert!(Instant::now() < deadline, "等待断开超时");
            let _ = server.poll(&verifier);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(server.state(), ConnectionState::WaitingClient);
    }
}
