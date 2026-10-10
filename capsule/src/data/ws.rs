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
        let mut token = cx.token.clone();
        let mut detail = Detail::Collapsed;
        let mut backoff_idx = 0;
        let mut force_reconnect = false;

        loop {
            // 处理积压命令
            match drain_cmds(&cx, &mut target, &mut token, &mut detail, &mut force_reconnect) {
                CmdFlow::Continue => {}
                CmdFlow::Shutdown => return,
            }

            set_status(&cx, ConnStatus::Connecting);
            match session(&cx, &mut target, &mut token, &mut detail, &mut force_reconnect) {
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
                    if sleep_or_shutdown(&cx, Duration::from_secs(wait), &mut target, &mut token) {
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
    token: &mut String,
    detail: &mut Detail,
    force_reconnect: &mut bool,
) -> CmdFlow {
    loop {
        match cx.cmd_rx.try_recv() {
            Ok(SourceCmd::SetDetail(d)) => *detail = d,
            Ok(SourceCmd::SetTarget(t, tok)) => {
                if *target != t || *token != tok {
                    *target = t;
                    *token = tok;
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
/// 收到 SetTarget 时写入新的地址与令牌并立即结束睡眠，返回 true 表示应退出线程。
fn sleep_or_shutdown(cx: &SourceCtx, total: Duration, target: &mut String, token: &mut String) -> bool {
    let step = Duration::from_millis(100);
    let mut slept = Duration::ZERO;
    while slept < total {
        match cx.cmd_rx.try_recv() {
            Ok(SourceCmd::Shutdown) => return true,
            Ok(SourceCmd::SetTarget(t, tok)) => {
                // 必须落盘新目标，否则重连仍用旧令牌（会导致认证失败无法恢复）
                *target = t;
                *token = tok;
                return false;
            }
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
    target: &mut String,
    token: &mut String,
    detail: &mut Detail,
    force_reconnect: &mut bool,
) -> SessionEnd {
    // 令牌为 base64 RawURLEncoding 字符集（A-Za-z0-9-_），query 安全无需编码
    let url = format!("ws://{target}/ws?token={token}");
    let (mut ws, _) = match connect(&url) {
        Ok(v) => v,
        Err(tungstenite::Error::Http(resp)) => {
            let code = resp.status().as_u16();
            if code == 401 || code == 403 {
                set_status(cx, ConnStatus::AuthFailed);
                // 认证失败不重连，阻塞等待用户改令牌或退出
                loop {
                    match cx.cmd_rx.recv() {
                        Ok(SourceCmd::SetTarget(t, tok)) => {
                            // 记录新地址/令牌，否则重连仍用旧值，会一直认证失败
                            *target = t;
                            *token = tok;
                            *force_reconnect = true;
                            return SessionEnd::Reconnect;
                        }
                        Ok(SourceCmd::Shutdown) => return SessionEnd::Shutdown,
                        Ok(SourceCmd::SetDetail(_)) => {}
                        Err(_) => return SessionEnd::Shutdown,
                    }
                }
            }
            return SessionEnd::Reconnect;
        }
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
            Ok(SourceCmd::SetTarget(t, tok)) => {
                // 记录新地址/令牌再重连，避免用旧令牌反复失败
                *target = t;
                *token = tok;
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
    let mut next = (*st.snap).clone();
    next.merge(&inc);
    st.snap = Arc::new(next);
    st.has_data = true;
    st.bump_seq();
    drop(st);
    post_update(cx.hwnd);
}
