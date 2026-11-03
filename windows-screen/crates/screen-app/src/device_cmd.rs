//! 3.5 英寸 CDC 真机验证命令（阶段 A/B 授权范围）。
//!
//! 不进入渲染主题链路，不发送未知命令，不读回，不发送 `0x79`。

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use screen_adapter_api::{DeviceAdapter, Transport};
use screen_adapter_turzx_cdc::{DiscoveryError, TurzxCdcAdapter, MODEL_ID};
use screen_transport_cdc::{CdcTransport, DeviceEvent, DeviceWatcher};

use crate::pattern::{self, Pattern};

fn adapter() -> TurzxCdcAdapter<CdcTransport> {
    TurzxCdcAdapter::new(CdcTransport::new())
}

fn map_discovery(err: DiscoveryError) -> String {
    format!("设备识别失败：{err}")
}

pub fn list_devices() -> Result<(), String> {
    let mut transport = CdcTransport::new();
    let all = transport.enumerate().map_err(|e| e.to_string())?;
    println!("enumerate: {} 个 COMPORT 接口候选", all.len());
    for c in &all {
        println!(
            "  key={} stable={:?} path={:?}",
            c.key, c.stable_id, c.open_path
        );
    }
    let expected = screen_adapter_turzx_cdc::expected_instance_id();
    let matched: Vec<_> = all.iter().filter(|c| c.key == expected).collect();
    println!("expected={expected}");
    println!("matched={}", matched.len());
    Ok(())
}

pub fn device_check() -> Result<(), String> {
    let mut adapter = adapter();
    let info = adapter.discover().map_err(map_discovery)?;
    println!("model={MODEL_ID}");
    println!("instance={}", info.key);
    println!("open_path={:?}", info.open_path);

    adapter.open(&info).map_err(|e| format!("打开失败：{e}"))?;
    println!("open=ok");
    let settings = adapter
        .transport()
        .current_settings()
        .map_err(|e| format!("读取串口参数失败：{e}"))?;
    println!(
        "params: dcb_len={} baud={} data={} parity={} stop={} dtr_ctrl={} rts_ctrl={} cts_flow={}",
        settings.dcb_length,
        settings.baud_rate,
        settings.byte_size,
        settings.parity,
        settings.stop_bits,
        settings.dtr_control,
        settings.rts_control,
        settings.outx_cts_flow
    );

    let report = adapter.init().map_err(|e| format!("初始化失败：{e}"))?;
    println!("init=ok notes={:?}", report.notes);
    adapter.close();
    println!("close=ok");
    Ok(())
}

pub fn send_test_frame(text: &str, repeat: u32) -> Result<(), String> {
    let pattern = Pattern::parse(text)
        .ok_or_else(|| format!("未知图案：{text}（允许 bars/gradient/solid）"))?;
    if repeat == 0 || repeat > 10 {
        return Err("repeat 必须在 1..=10".to_string());
    }
    let frame = pattern::build(pattern)?;
    let mut adapter = adapter();
    let info = adapter.discover().map_err(map_discovery)?;
    println!("instance={}", info.key);
    adapter.open(&info).map_err(|e| format!("打开失败：{e}"))?;
    adapter.init().map_err(|e| format!("初始化失败：{e}"))?;
    println!("pattern={text} repeat={repeat}");

    for i in 0..repeat {
        let start = Instant::now();
        adapter
            .send_frame(&frame)
            .map_err(|e| format!("发送第 {} 帧失败：{e}", i + 1))?;
        let report = adapter.last_report().ok_or("缺少发送报告")?;
        let elapsed_secs = start.elapsed().as_secs_f64();
        let rate = if elapsed_secs > 0.0 {
            report.bytes as f64 / elapsed_secs / 1024.0
        } else {
            0.0
        };
        println!(
            "frame {}: header={:02X?} bytes={} chunks={} last_chunk={} elapsed_ms={} write_us(header={} chunks_total={} avg={} min={} max={}) rate_kib_s={:.1}",
            i + 1,
            report.header,
            report.bytes,
            report.chunks,
            report.last_chunk_len,
            start.elapsed().as_millis(),
            report.header_us,
            report.chunks_total_us,
            report.chunk_avg_us,
            report.chunk_min_us,
            report.chunk_max_us,
            rate
        );

        let q_before = adapter.transport().output_queue_bytes().ok();
        let flush = adapter.transport().flush_with_report();
        let q_after = adapter.transport().output_queue_bytes().ok();
        println!(
            "flush: started_epoch_ms={} elapsed_us={} ok={} error={:?} queue_before={:?} queue_after={:?}",
            flush.started_epoch_ms, flush.elapsed_us, flush.ok(), flush.error, q_before, q_after
        );
        if let Some(err) = flush.error {
            adapter.close();
            println!("close=cleanup-after-flush-error");
            return Err(format!("FlushFileBuffers 失败：{err}"));
        }
    }

    // 完整帧 + Flush 成功后，保持句柄打开 3 秒再关闭（本轮唯一变量）。
    let hold_start = Instant::now();
    let hold_started_epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    std::thread::sleep(Duration::from_secs(3));
    let hold_ended_epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    println!(
        "hold: started_epoch_ms={} ended_epoch_ms={} elapsed_ms={}",
        hold_started_epoch_ms,
        hold_ended_epoch_ms,
        hold_start.elapsed().as_millis()
    );

    adapter.close();
    println!("close=ok");
    Ok(())
}

pub fn watch_devices(seconds: u64) -> Result<(), String> {
    if !(5..=300).contains(&seconds) {
        return Err("watch 秒数必须在 5..=300".to_string());
    }
    let watcher = DeviceWatcher::register().map_err(|e| format!("注册通知失败：{e}"))?;
    println!("watch start: {}s（不打开设备）", seconds);
    let start = Instant::now();
    let mut events = 0u32;
    while start.elapsed() < Duration::from_secs(seconds) {
        if let Some(event) = watcher.recv_timeout(Duration::from_millis(500)) {
            events += 1;
            println!("event {:?} at {}s", event, start.elapsed().as_secs());
        }
    }
    println!("watch end: events={events}");
    Ok(())
}

pub fn device_session(seconds: u64) -> Result<(), String> {
    if !(5..=600).contains(&seconds) {
        return Err("session 秒数必须在 5..=600".to_string());
    }
    let frame = pattern::build(Pattern::Bars)?;
    let mut adapter = adapter();
    let watcher = DeviceWatcher::register().map_err(|e| format!("注册通知失败：{e}"))?;
    let info = adapter.discover().map_err(map_discovery)?;
    println!("session start: instance={}", info.key);
    adapter.open(&info).map_err(|e| format!("打开失败：{e}"))?;
    adapter.init().map_err(|e| format!("初始化失败：{e}"))?;

    let start = Instant::now();
    let mut frames = 0u32;
    let mut recoveries = 0u32;
    let mut dropped = 0u32;

    while start.elapsed() < Duration::from_secs(seconds) {
        if let Some(DeviceEvent::Removal) = watcher.try_recv() {
            println!("CM removal at {}s", start.elapsed().as_secs());
            adapter.close();
            match recover(&mut adapter, &watcher, Duration::from_secs(30)) {
                Ok(()) => {
                    recoveries += 1;
                    println!("recovered at {}s", start.elapsed().as_secs());
                }
                Err(msg) => {
                    println!("recovery failed: {msg}");
                    break;
                }
            }
        }
        match adapter.send_frame(&frame) {
            Ok(()) => {
                frames += 1;
                if frames % 5 == 1 {
                    let report = adapter.last_report().ok_or("缺少发送报告")?;
                    println!(
                        "t={}s frames={} header={:02X?} chunks={} last={}",
                        start.elapsed().as_secs(),
                        frames,
                        report.header,
                        report.chunks,
                        report.last_chunk_len
                    );
                }
            }
            Err(err) => {
                dropped += 1;
                println!("send failed at {}s: {err}", start.elapsed().as_secs());
                adapter.close();
                match recover(&mut adapter, &watcher, Duration::from_secs(30)) {
                    Ok(()) => {
                        recoveries += 1;
                        println!("recovered at {}s", start.elapsed().as_secs());
                    }
                    Err(msg) => {
                        println!("recovery failed: {msg}");
                        break;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    adapter.close();
    println!("session end: frames={frames} dropped={dropped} recoveries={recoveries}");
    Ok(())
}

fn recover(
    adapter: &mut TurzxCdcAdapter<CdcTransport>,
    watcher: &DeviceWatcher,
    timeout: Duration,
) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(info) = adapter.discover() {
            if adapter.open(&info).is_ok() && adapter.init().is_ok() {
                return Ok(());
            }
        }
        match watcher.recv_timeout(Duration::from_millis(500)) {
            Some(event) => println!("wait event: {:?}", event),
            None => {}
        }
    }
    Err("30s 内未恢复".to_string())
}
