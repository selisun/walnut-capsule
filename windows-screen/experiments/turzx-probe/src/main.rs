//! E0 基准实验探针：只复现当前路径 A，不包含 0x79/0x6C/0x67。
//! 独立实验程序，不修改生产代码。

use std::fs;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use screen_adapter_api::{PortCandidate, Transport};
use screen_protocol::{
    brightness_zero, chunk_ranges, command, frame_header, CMD_SCREEN_ON, LEGACY_CHUNK_SIZE,
};
use screen_transport_cdc::CdcTransport;

const EXPECTED_KEY: &str = "USB\\VID_1A86&PID_5722\\USB35INCHIPSV2";
const EXPECTED_STABLE: &str = "USB35INCHIPSV2";
const DEFAULT_FRAME: &str =
    r"D:\nas胶囊\windows-screen\target\experiment-plan\frame_480x320.rgb565.bin";

fn epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let frame_path = args.next().unwrap_or_else(|| DEFAULT_FRAME.to_string());
    let width: u32 = args
        .next()
        .map(|v| v.parse().map_err(|_| format!("width 非法：{v}")))
        .transpose()?
        .unwrap_or(480);
    let height: u32 = args
        .next()
        .map(|v| v.parse().map_err(|_| format!("height 非法：{v}")))
        .transpose()?
        .unwrap_or(320);
    if (width as usize) * (height as usize) * 2 != 307_200 {
        return Err(format!("尺寸与像素长度不符：{width}x{height}"));
    }
    let frame = fs::read(&frame_path)
        .map_err(|e| format!("读取帧失败 {frame_path}：{e}"))?;
    if frame.len() != 307_206 {
        return Err(format!("完整帧长度不符：{}，预期 307206", frame.len()));
    }
    let header = frame_header(width, height).map_err(|e| format!("帧头失败：{e}"))?;
    if frame[..6] != header {
        return Err(format!(
            "文件帧头不符：{:02X?}，预期 {:02X?}",
            &frame[..6],
            header
        ));
    }
    let pixels = &frame[6..];
    if pixels.len() != 307_200 {
        return Err(format!("像素长度不符：{}，预期 307200", pixels.len()));
    }
    println!(
        "probe: start_epoch_ms={} frame_path={} dims={}x{} full_len={} header={:02X?} pixels={} first16={:02X?} last16={:02X?}",
        epoch_ms(),
        frame_path,
        width,
        height,
        frame.len(),
        header,
        pixels.len(),
        &pixels[..16],
        &pixels[pixels.len() - 16..]
    );

    let mut transport = CdcTransport::new();
    let candidates = transport
        .enumerate()
        .map_err(|e| format!("枚举失败：{e}"))?;
    println!("enumerate: {} candidates", candidates.len());
    for c in &candidates {
        println!("candidate: key={} stable={:?} path={:?}", c.key, c.stable_id, c.open_path);
    }
    let matches: Vec<PortCandidate> = candidates
        .into_iter()
        .filter(|c| c.key == EXPECTED_KEY)
        .collect();
    if matches.len() != 1 {
        return Err(format!("精确匹配数量为 {}，要求 1；停止", matches.len()));
    }
    let candidate = matches.into_iter().next().expect("len == 1");
    if candidate.stable_id.as_deref() != Some(EXPECTED_STABLE) {
        return Err(format!("stable_id 不符：{:?}", candidate.stable_id));
    }
    if candidate.open_path.is_none() {
        return Err("缺少 open_path；停止".to_string());
    }
    println!(
        "matched: instance={} stable={:?} path={:?}",
        candidate.key, candidate.stable_id, candidate.open_path
    );

    transport
        .open(&candidate)
        .map_err(|e| format!("独占打开失败：{e}"))?;
    println!("open=ok");
    let settings = transport
        .current_settings()
        .map_err(|e| format!("读取 DCB 失败：{e}"))?;
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

    let result = run_sequence(&mut transport, pixels, header);
    let close_result = result.and_then(|()| {
        transport.close();
        println!("close=ok");
        Ok(())
    });
    if close_result.is_err() {
        transport.close();
    }
    close_result
}

fn run_sequence(transport: &mut CdcTransport, frame: &[u8], header: [u8; 6]) -> Result<(), String> {
    // 当前路径 A：2s → 0x6D → 60ms → 0x6E(0) → 60ms
    sleep(Duration::from_millis(2_000));
    let on = command(CMD_SCREEN_ON).map_err(|e| format!("构造 0x6D 失败：{e}"))?;
    let start = Instant::now();
    let written = transport
        .write(&on, 3_000)
        .map_err(|e| format!("写 0x6D 失败：{e}"))?;
    println!(
        "init: cmd={:02X?} len={} written={} us={}",
        on,
        on.len(),
        written,
        start.elapsed().as_micros()
    );
    sleep(Duration::from_millis(60));

    let brightness = brightness_zero();
    let start = Instant::now();
    let written = transport
        .write(&brightness, 3_000)
        .map_err(|e| format!("写 0x6E 失败：{e}"))?;
    println!(
        "init: cmd={:02X?} len={} written={} us={}",
        brightness,
        brightness.len(),
        written,
        start.elapsed().as_micros()
    );
    sleep(Duration::from_millis(60));

    println!("header={:02X?}", header);
    let ranges = chunk_ranges(frame.len(), LEGACY_CHUNK_SIZE)
        .map_err(|e| format!("分块失败：{e}"))?;
    if ranges.len() != 38 || ranges.last().map(|r| r.len()) != Some(4_096) {
        return Err(format!("分块不符：{} 块，末块 {:?}", ranges.len(), ranges.last()));
    }
    println!(
        "chunks={} last_chunk={} total_len={}",
        ranges.len(),
        ranges.last().map(|r| r.len()).unwrap_or(0),
        frame.len()
    );
    let q_before = transport.output_queue_bytes().ok();

    let frame_start = Instant::now();
    let header_start = Instant::now();
    let written = transport
        .write(&header, 3_000)
        .map_err(|e| format!("写帧头失败：{e}"))?;
    println!(
        "header_write: len={} written={} us={}",
        header.len(),
        written,
        header_start.elapsed().as_micros()
    );

    let mut chunk_us = Vec::with_capacity(ranges.len());
    for (i, range) in ranges.iter().enumerate() {
        let chunk_start = Instant::now();
        let chunk = &frame[range.clone()];
        let written = transport
            .write(chunk, 3_000)
            .map_err(|e| format!("写第 {i} 块失败：{e}"))?;
        let us = chunk_start.elapsed().as_micros();
        chunk_us.push(us);
        println!(
            "chunk: i={} len={} written={} us={}",
            i,
            chunk.len(),
            written,
            us
        );
        if written != chunk.len() {
            return Err(format!("第 {i} 块写入不完整：{written}/{}", chunk.len()));
        }
    }
    let total_us = frame_start.elapsed().as_micros();
    let chunks_total: u128 = chunk_us.iter().sum();
    let q_mid = transport.output_queue_bytes().ok();
    let flush = transport.flush_with_report();
    let q_after = transport.output_queue_bytes().ok();
    println!(
        "frame: elapsed_ms={} header_and_chunks_us={} chunks_total_us={} avg={} min={} max={}",
        total_us / 1_000,
        total_us,
        chunks_total,
        chunks_total / chunk_us.len() as u128,
        chunk_us.iter().min().unwrap_or(&0),
        chunk_us.iter().max().unwrap_or(&0)
    );
    println!(
        "flush: started_epoch_ms={} elapsed_us={} ok={} error={:?} queue_before={:?} queue_mid={:?} queue_after={:?}",
        flush.started_epoch_ms,
        flush.elapsed_us,
        flush.ok(),
        flush.error,
        q_before,
        q_mid,
        q_after
    );
    if let Some(err) = flush.error {
        return Err(format!("FlushFileBuffers 失败：{err}"));
    }
    Ok(())
}
