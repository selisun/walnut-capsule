//! B-10 黄金字节对端校验：capsule 侧编码器（`capsule/src/screen_bridge.rs`）产出的字节，
//! 必须能被本 crate 的 `Header` / `Message` 正常解析，且字段与 fixture 一致。
//!
//! 两侧共享同一份 `GOLDEN_JSON` 常量。任一侧改动字段名/顺序/头布局，本测试或
//! capsule 侧 `golden_json_is_stable_and_cross_checked` 会立刻失败。

#![cfg(windows)]

use screen_ipc::{Header, Message, MessageKind, HEADER_LEN, SCHEMA_MAJOR};

/// 与 `capsule/src/screen_bridge.rs` 中的 `GOLDEN_JSON` **必须逐字节一致**。
const GOLDEN_JSON: &str = "{\"seq\":42,\"ts_ms\":1700000000000,\"nas\":{\"name\":\"nas\",\"online\":true},\"metrics\":{\"cpu\":{\"usage_pct\":12.5,\"temp_c\":45.0,\"vendor\":\"intel\",\"load_1m\":0.5,\"load_5m\":0.4,\"load_15m\":0.3},\"memory\":{\"usage_pct\":55.0,\"used_bytes\":8589934592,\"total_bytes\":17179869184,\"cached_bytes\":2147483648,\"available_bytes\":8589934592},\"gpu\":{\"name\":\"card0\",\"vendor\":\"amd\",\"usage_pct\":30.0,\"temp_c\":50.0,\"vram_used_bytes\":1073741824,\"vram_total_bytes\":8589934592},\"network\":{\"iface\":\"eth0\",\"up_bps\":200,\"down_bps\":100},\"disk\":{\"device\":\"/dev/sda\",\"read_bps\":1024,\"write_bps\":2048},\"storage\":[{\"name\":\"vol1\",\"used_bytes\":1000,\"total_bytes\":2000}]}}";

/// 按 capsule 编码器的字节布局拼装（不使用 `Header::encode`，避免「自证」）。
fn capsule_bytes(seq: u64, json: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"HTSP");
    out.extend_from_slice(&SCHEMA_MAJOR.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.push(3); // Snapshot
    out.push(0); // flags
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(json.as_bytes());
    out
}

#[test]
fn capsule_bytes_parse_with_reference_header() {
    let bytes = capsule_bytes(42, GOLDEN_JSON);
    assert_eq!(bytes.len(), HEADER_LEN + GOLDEN_JSON.len());
    let header = Header::decode(&bytes).expect("header decode");
    assert_eq!(header.kind, MessageKind::Snapshot);
    assert_eq!(header.seq, 42);
    assert_eq!(header.payload_len as usize, GOLDEN_JSON.len());
    assert_eq!(header.version.major, SCHEMA_MAJOR);
}

#[test]
fn capsule_payload_deserializes_into_screen_snapshot() {
    let bytes = capsule_bytes(42, GOLDEN_JSON);
    let message = Message::from_bytes(&bytes).expect("message");
    let snapshot: screen_core::Snapshot = message.decode_payload().expect("payload");
    assert_eq!(snapshot.seq, 42);
    assert_eq!(snapshot.ts_ms, 1_700_000_000_000);
    assert_eq!(snapshot.nas.name.as_deref(), Some("nas"));
    assert_eq!(snapshot.nas.online, Some(true));

    let cpu = snapshot.metrics.cpu.expect("cpu");
    assert_eq!(cpu.usage_pct, Some(12.5));
    assert_eq!(cpu.temp_c, Some(45.0));
    assert_eq!(cpu.load_1m, Some(0.5));
    assert_eq!(cpu.load_30m, None);

    let memory = snapshot.metrics.memory.expect("memory");
    assert_eq!(memory.used_bytes, Some(8_589_934_592));
    assert_eq!(memory.total_bytes, Some(17_179_869_184));

    let gpu = snapshot.metrics.gpu.expect("gpu");
    assert_eq!(gpu.name.as_deref(), Some("card0"));

    let net = snapshot.metrics.network.expect("network");
    assert_eq!(net.up_bps, Some(200));
    assert_eq!(net.down_bps, Some(100));

    let disk = snapshot.metrics.disk.expect("disk");
    assert_eq!(disk.device.as_deref(), Some("/dev/sda"));

    assert_eq!(snapshot.metrics.storage.len(), 1);
}

#[test]
fn reference_encoder_matches_capsule_layout() {
    let reference = Header::new(MessageKind::Snapshot, 42, GOLDEN_JSON.len())
        .expect("header")
        .encode();
    let bytes = capsule_bytes(42, GOLDEN_JSON);
    assert_eq!(&bytes[..HEADER_LEN], &reference);
}
