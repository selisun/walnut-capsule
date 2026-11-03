//! 跨 crate 集成冒烟测试：配置 → 主题 → 布局 → 渲染 → 帧；IPC 快照结构往返。

use screen_app::{Config, ScreenApp};
use screen_core::{Orientation, ScreenProfile, Snapshot};
use screen_ipc::{Message, MessageKind};
use screen_layout::{build, TemplateKind};

#[test]
fn end_to_end_skeleton_without_device_or_data() {
    let missing = std::env::temp_dir().join("hetao-screen-smoke-missing.toml");
    let _ = std::fs::remove_file(&missing);
    let app = ScreenApp::init(Some(&missing)).expect("init");
    assert_eq!(app.devices().active_count(), 0);

    let profile = ScreenProfile::new(
        "bench-3.5-landscape",
        480,
        320,
        Orientation::Landscape,
        true,
    )
    .expect("profile");
    let layout = build(
        &profile,
        &app.config().components.visible,
        TemplateKind::Landscape4Col,
        app.config().theme.font_scale,
    )
    .expect("layout");
    assert_eq!(layout.items.len(), 5);

    let frame = app.render_probe(&profile).expect("render");
    assert_eq!(frame.width(), 480);
    assert_eq!(frame.height(), 320);
    assert_eq!(frame.pixels().len(), 480 * 320 * 4);
}

#[test]
fn ipc_snapshot_message_roundtrip() {
    let snapshot = Snapshot::new(42, 1_000);
    let message = Message::with_payload(MessageKind::Snapshot, 42, &snapshot).expect("encode");
    let bytes = message.to_bytes().expect("bytes");
    let decoded = Message::from_bytes(&bytes).expect("decode");
    let back: Snapshot = decoded.decode_payload().expect("payload");
    assert_eq!(back, snapshot);
}

#[test]
fn config_has_no_nas_credentials_fields() {
    let text = toml::to_string_pretty(&Config::default()).expect("toml");
    for forbidden in ["token", "password", "host", "port"] {
        assert!(!text.contains(forbidden), "配置中不应出现 {forbidden}");
    }
}
