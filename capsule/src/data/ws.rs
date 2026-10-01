//! WsSource：连接 NAS 的 /ws，订阅推送，断线自动重连（1s/2s/5s/10s）。

use super::*;
use std::io::ErrorKind;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message};

const BACKOFF: &[u64] = &[1, 2, 5, 10];
/// 读超时：兼顾“空闲近 0 CPU”与命令响应延迟。
const READ_TIMEOUT: Duration = Duration::from_millis(500);

pub struct WsSource;

impl DataSource for WsSource {
    fn run(self, cx: SourceCtx) {
        let mut target = cx.target.clone();
        let mut detail = Detail::Collapsed;
        let mut backoff_idx = 0;
        let mut force_reconnect = false;

        loop {
            // 处理积压命令
            match drain_cmds(&cx, &mut target, &mut detail, &mut force_reconnect) {
                CmdFlow::Continue => {}
                CmdFlow::Shutdown => return,
            }

            set_status(&cx, ConnStatus::Connecting);
            match session(&cx, &target, &mut detail, &mut force_reconnect) {
                SessionEnd::Shutdown => return,
                SessionEnd::Reconnect => {
                    if force_reconnect {
                        force_reconnect = false;
                        backoff_idx = 0;
                        continue;
                    }
                    let wait = BACKOFF[backoff_idx.min(BACKOFF.len() - 1)];
                    backoff_idx += 1;
                    set_status(&cx, ConnStatus::Reconnect(wait));
                    if sleep_or_shutdown(&cx, Duration::from_secs(wait)) {
                        return;
                    }
                }
            }
        }
    }
}

enum CmdFlow {
    Continue,
    Shutdown,
}

enum SessionEnd {
    Reconnect,
    Shutdown,
}

fn drain_cmds(
    cx: &SourceCtx,
    target: &mut String,
    detail: &mut Detail,
    force_reconnect: &mut bool,
) -> CmdFlow {
    loop {
        match cx.cmd_rx.try_recv() {
            Ok(SourceCmd::SetDetail(d)) => *detail = d,
            Ok(SourceCmd::SetTarget(t)) => {
                if *target != t {
                    *target = t;
                    *force_reconnect = true;
                }
            }
            Ok(SourceCmd::Shutdown) => return CmdFlow::Shutdown,
            Err(TryRecvError::Empty) => return CmdFlow::Continue,
            Err(TryRecvError::Disconnected) => return CmdFlow::Shutdown,
        }
    }
}

fn set_status(cx: &SourceCtx, status: ConnStatus) {
    let mut st = cx.state.write().unwrap();
    st.conn = status;
    st.bump_seq();
    post_update(cx.hwnd);
}

/// 可中断的睡眠：期间响应 Shutdown / SetTarget（提前醒来重连）。
/// 返回 true 表示应退出线程。
fn sleep_or_shutdown(cx: &SourceCtx, total: Duration) -> bool {
    let step = Duration::from_millis(100);
    let mut slept = Duration::ZERO;
    while slept < total {
        match cx.cmd_rx.try_recv() {
            Ok(SourceCmd::Shutdown) => return true,
            Ok(SourceCmd::SetTarget(_)) => return false, // 立即重连（drain 时生效）
            Ok(SourceCmd::SetDetail(_)) => {}
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => return true,
        }
        std::thread::sleep(step);
        slept += step;
    }
    false
}

fn subscribe_msg(detail: Detail) -> String {
    match detail {
        Detail::Collapsed => serde_json::json!({
            "type": "subscribe",
            "metrics": ["cpu.usage", "gpu.usage", "mem.usage", "disk.io", "net.io"],
            "interval": 1000
        })
        .to_string(),
        Detail::Expanded => serde_json::json!({
            "type": "subscribe",
            "interval": 1000
        })
        .to_string(),
    }
}

/// 单次连接会话。返回时连接已断开或收到命令。
fn session(
    cx: &SourceCtx,
    target: &str,
    detail: &mut Detail,
    force_reconnect: &mut bool,
) -> SessionEnd {
    let url = format!("ws://{target}/ws");
    let (mut ws, _) = match connect(&url) {
        Ok(v) => v,
        Err(_) => return SessionEnd::Reconnect,
    };
    if let MaybeTlsStream::Plain(tcp) = ws.get_mut() {
        let _ = tcp.set_read_timeout(Some(READ_TIMEOUT));
        let _ = tcp.set_nodelay(true);
    }
    if ws.send(Message::Text(subscribe_msg(*detail))).is_err() {
        return SessionEnd::Reconnect;
    }
    set_status(cx, ConnStatus::Online);

    loop {
        match ws.read() {
            Ok(Message::Text(txt)) => handle_push(cx, &txt),
            Ok(Message::Close(_)) => return SessionEnd::Reconnect,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return SessionEnd::Reconnect,
        }
        // 每个读周期检查一次命令
        match cx.cmd_rx.try_recv() {
            Ok(SourceCmd::SetDetail(d)) => {
                *detail = d;
                if ws.send(Message::Text(subscribe_msg(d))).is_err() {
                    return SessionEnd::Reconnect;
                }
            }
            Ok(SourceCmd::SetTarget(_)) => {
                *force_reconnect = true;
                return SessionEnd::Reconnect;
            }
            Ok(SourceCmd::Shutdown) => return SessionEnd::Shutdown,
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => return SessionEnd::Shutdown,
        }
    }
}

fn handle_push(cx: &SourceCtx, txt: &str) {
    let inc: Snapshot = match serde_json::from_str(txt) {
        Ok(v) => v,
        Err(_) => return, // 坏包丢弃，等下一帧
    };
    let mut st = cx.state.write().unwrap();
    st.snap.merge(&inc);
    st.has_data = true;
    st.bump_seq();
    post_update(cx.hwnd);
}
