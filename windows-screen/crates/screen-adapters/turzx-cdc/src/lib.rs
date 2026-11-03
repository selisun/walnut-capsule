//! 图灵智显 3.5 英寸 CDC 适配器。
//!
//! 只实现 E1-A 真机验证的物理基线：
//! 320×480、RGB565LE、6 字节帧头 `00 00 04 FD DF C5`、8192 分块、`0x6D`/`0x6E`（0 值）。
//! **不发送 `0x79`/`0x6C`/`0x67`，不实现未知亮度档位，不读取回。**
//! 横屏由 App 在软件层旋转为 320×480 物理帧，适配器不接受 480×320。

use screen_adapter_api::{
    AdapterError, DeviceAdapter, DeviceInfo, Health, PortCandidate, Transport,
};
use screen_core::{
    Cap, DeviceCapabilities, Frame, Orientation, PixelFormat, RefreshMode, Rotation, Source,
    TransportKind,
};
use screen_protocol::{
    brightness_zero, chunk_ranges, command, frame_header, CMD_SCREEN_ON, LEGACY_CHUNK_SIZE,
};

pub const MODEL_ID: &str = "turzx-3.5-cdc";
pub const VENDOR_ID: &str = "1A86";
pub const PRODUCT_ID: &str = "5722";
pub const INSTANCE_SUFFIX: &str = "USB35INCHIPSV2";
pub const DEVICE_WIDTH: u32 = 320;
pub const DEVICE_HEIGHT: u32 = 480;

/// `USB\VID_1A86&PID_5722\USB35INCHIPSV2`（PnP 实例路径，不是 USB 序列号）。
pub fn expected_instance_id() -> String {
    format!("USB\\VID_{VENDOR_ID}&PID_{PRODUCT_ID}\\{INSTANCE_SUFFIX}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryError {
    NotFound,
    Multiple(Vec<String>),
    AmbiguousInstance(String),
    NotOpenable(String),
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryError::NotFound => write!(f, "未找到符合实例 ID 的 3.5 英寸设备"),
            DiscoveryError::Multiple(ids) => {
                write!(f, "发现多个匹配设备，拒绝自动选择：{}", ids.join(", "))
            }
            DiscoveryError::AmbiguousInstance(id) => {
                write!(f, "实例后缀冲突：{id}")
            }
            DiscoveryError::NotOpenable(id) => write!(f, "设备缺少可打开接口路径：{id}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendReport {
    pub header: [u8; 6],
    pub bytes: usize,
    pub chunks: usize,
    pub last_chunk_len: usize,
    pub header_us: u128,
    pub chunks_total_us: u128,
    pub chunk_min_us: u128,
    pub chunk_max_us: u128,
    pub chunk_avg_us: u128,
}

pub struct TurzxCdcAdapter<T: Transport> {
    transport: T,
    capabilities: DeviceCapabilities,
    init_delay_ms: u64,
    is_open: bool,
    last_report: Option<SendReport>,
}

impl<T: Transport> TurzxCdcAdapter<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            capabilities: baseline_capabilities(),
            init_delay_ms: 2_000,
            is_open: false,
            last_report: None,
        }
    }

    /// 仅用于单元测试，缩短旧系统的 2s 等待。
    pub fn with_init_delay_ms(mut self, delay_ms: u64) -> Self {
        self.init_delay_ms = delay_ms;
        self
    }

    pub fn transport(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn last_report(&self) -> Option<&SendReport> {
        self.last_report.as_ref()
    }

    /// 枚举并筛选；只有唯一一个完全匹配的实例才返回。
    pub fn discover(&mut self) -> Result<DeviceInfo, DiscoveryError> {
        let candidates = self
            .transport
            .enumerate()
            .map_err(|_| DiscoveryError::NotFound)?;
        let expected = expected_instance_id();
        let matches: Vec<PortCandidate> = candidates
            .into_iter()
            .filter(|c| c.key == expected)
            .collect();

        if matches.is_empty() {
            return Err(DiscoveryError::NotFound);
        }
        if matches.len() > 1 {
            return Err(DiscoveryError::Multiple(
                matches.iter().map(|c| c.key.clone()).collect(),
            ));
        }
        let candidate = matches.into_iter().next().expect("len == 1");
        let suffix_ok = candidate
            .stable_id
            .as_deref()
            .map(|s| s == INSTANCE_SUFFIX)
            .unwrap_or(false);
        if !suffix_ok {
            return Err(DiscoveryError::AmbiguousInstance(candidate.key));
        }
        let open_path = candidate
            .open_path
            .clone()
            .ok_or_else(|| DiscoveryError::NotOpenable(candidate.key.clone()))?;
        Ok(DeviceInfo {
            key: candidate.key,
            model_id: MODEL_ID.to_string(),
            stable_id: candidate.stable_id,
            transport: TransportKind::Cdc,
            open_path: Some(open_path),
        })
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), AdapterError> {
        // 串口 WriteFile 可能返回部分写入；循环写直到完整，超时/零进度即失败。
        // 不改变单块大小、Flush 或协议时序，只保证每次发送不被静默截断。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut offset = 0usize;
        while offset < bytes.len() {
            if std::time::Instant::now() >= deadline {
                return Err(AdapterError::Timeout);
            }
            let remaining = bytes.len() - offset;
            let written = self.transport.write(&bytes[offset..], 3_000)?;
            if written == 0 {
                return Err(AdapterError::Io("写入返回 0 字节".to_string()));
            }
            if written > remaining {
                return Err(AdapterError::Io(format!(
                    "写入长度超出：{written}/{remaining}"
                )));
            }
            offset += written;
        }
        Ok(())
    }
}

impl<T: Transport> DeviceAdapter for TurzxCdcAdapter<T> {
    fn model_id(&self) -> &str {
        MODEL_ID
    }

    fn capabilities(&self) -> &DeviceCapabilities {
        &self.capabilities
    }

    fn open(&mut self, device: &DeviceInfo) -> Result<(), AdapterError> {
        if device.model_id != MODEL_ID {
            return Err(AdapterError::Unsupported("非 3.5 英寸型号"));
        }
        let open_path = device
            .open_path
            .clone()
            .ok_or(AdapterError::Unsupported("缺少接口路径"))?;
        let candidate = PortCandidate {
            key: device.key.clone(),
            description: device.key.clone(),
            transport: TransportKind::Cdc,
            stable_id: device.stable_id.clone(),
            open_path: Some(open_path),
        };
        self.transport.open(&candidate)?;
        self.is_open = true;
        Ok(())
    }

    fn init(&mut self) -> Result<screen_adapter_api::InitReport, AdapterError> {
        if !self.is_open {
            return Err(AdapterError::Disconnected);
        }
        std::thread::sleep(std::time::Duration::from_millis(self.init_delay_ms));
        let on = command(CMD_SCREEN_ON).map_err(|_| AdapterError::Unsupported("开屏命令"))?;
        self.write_all(&on)?;
        std::thread::sleep(std::time::Duration::from_millis(60));
        let brightness = brightness_zero();
        self.write_all(&brightness)?;
        std::thread::sleep(std::time::Duration::from_millis(60));
        Ok(screen_adapter_api::InitReport {
            firmware: None,
            notes: vec!["初始化使用旧系统组合级基线：0x6D 开屏、0x6E 亮度（0 值）".to_string()],
        })
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<(), AdapterError> {
        if !self.is_open {
            return Err(AdapterError::Disconnected);
        }
        if frame.width != DEVICE_WIDTH
            || frame.height != DEVICE_HEIGHT
            || frame.pixel_format != PixelFormat::Rgb565Le
        {
            return Err(AdapterError::Unsupported("帧必须是 320×480 RGB565LE"));
        }
        if frame.pixels.len() != (DEVICE_WIDTH * DEVICE_HEIGHT * 2) as usize {
            return Err(AdapterError::Io("帧长度不符".to_string()));
        }

        let header = frame_header(frame.width, frame.height)
            .map_err(|_| AdapterError::Unsupported("帧头尺寸"))?;
        let header_start = std::time::Instant::now();
        self.write_all(&header)?;
        let header_us = header_start.elapsed().as_micros();

        let ranges = chunk_ranges(frame.pixels.len(), LEGACY_CHUNK_SIZE)
            .map_err(|_| AdapterError::Unsupported("分块"))?;
        let mut chunk_us = Vec::with_capacity(ranges.len());
        for range in &ranges {
            let start = std::time::Instant::now();
            self.write_all(&frame.pixels[range.clone()])?;
            chunk_us.push(start.elapsed().as_micros());
        }
        let chunks_total_us: u128 = chunk_us.iter().sum();
        let chunk_min_us = *chunk_us.iter().min().unwrap_or(&0);
        let chunk_max_us = *chunk_us.iter().max().unwrap_or(&0);
        let chunk_avg_us = if chunk_us.is_empty() {
            0
        } else {
            chunks_total_us / chunk_us.len() as u128
        };
        self.last_report = Some(SendReport {
            header,
            bytes: header.len() + frame.pixels.len(),
            chunks: ranges.len(),
            last_chunk_len: ranges.last().map(|r| r.len()).unwrap_or(0),
            header_us,
            chunks_total_us,
            chunk_min_us,
            chunk_max_us,
            chunk_avg_us,
        });
        Ok(())
    }

    fn set_brightness(&mut self, level: u8) -> Result<(), AdapterError> {
        if level != 0 {
            return Err(AdapterError::Unsupported("亮度档位未验证，仅允许 0"));
        }
        if !self.is_open {
            return Err(AdapterError::Disconnected);
        }
        let bytes = brightness_zero();
        self.write_all(&bytes)
    }

    fn poll_health(&mut self) -> Result<Health, AdapterError> {
        if !self.is_open {
            return Err(AdapterError::Disconnected);
        }
        Ok(Health::ok())
    }

    fn close(&mut self) {
        self.transport.close();
        self.is_open = false;
    }
}

impl<T: Transport> Drop for TurzxCdcAdapter<T> {
    fn drop(&mut self) {
        if self.is_open {
            self.close();
        }
    }
}

fn baseline_capabilities() -> DeviceCapabilities {
    DeviceCapabilities {
        model_id: MODEL_ID.to_string(),
        transport: TransportKind::Cdc,
        frame_width: Cap::Known(DEVICE_WIDTH),
        frame_height: Cap::Known(DEVICE_HEIGHT),
        native_orientation: Cap::Known(Orientation::Portrait),
        rotation_modes: Cap::Known(vec![Rotation::None]),
        mirror: Cap::Known(false),
        invert: Cap::Known(false),
        pixel_format: Cap::Known(PixelFormat::Rgb565Le),
        row_padding: Cap::Known(0),
        max_payload: Cap::Known(LEGACY_CHUNK_SIZE),
        chunk_size: Cap::Known(LEGACY_CHUNK_SIZE),
        refresh_mode: Cap::Known(RefreshMode::Full),
        supports_readback: Cap::Known(false),
        brightness: Cap::Unknown, // 档位编码未验证
        verified: true,
        source: Source::CombinationMachineVerified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use screen_adapter_api::{AdapterError, Transport};
    use screen_core::{Frame, PixelFormat, TransportKind};

    #[derive(Default)]
    struct MockTransport {
        candidates: Vec<PortCandidate>,
        writes: Vec<Vec<u8>>,
        opened: bool,
        fail_write_at: Option<usize>,
        fail_write_timeout_at: Option<usize>,
        partial_write_at: Option<usize>,
        partial_len: usize,
        partial_done: bool,
        open_fails_busy: bool,
    }

    impl Transport for MockTransport {
        fn kind(&self) -> TransportKind {
            TransportKind::Cdc
        }
        fn enumerate(&mut self) -> Result<Vec<PortCandidate>, AdapterError> {
            Ok(self.candidates.clone())
        }
        fn open(&mut self, _candidate: &PortCandidate) -> Result<(), AdapterError> {
            if self.open_fails_busy {
                return Err(AdapterError::Busy);
            }
            self.opened = true;
            Ok(())
        }
        fn write(&mut self, bytes: &[u8], _timeout_ms: u32) -> Result<usize, AdapterError> {
            if let Some(limit) = self.fail_write_at {
                if self.writes.len() >= limit {
                    return Err(AdapterError::Disconnected);
                }
            }
            if let Some(limit) = self.fail_write_timeout_at {
                if self.writes.len() >= limit {
                    return Err(AdapterError::Timeout);
                }
            }
            if !self.opened {
                return Err(AdapterError::Disconnected);
            }
            if !self.partial_done {
                if let Some(at) = self.partial_write_at {
                    if self.writes.len() == at {
                        let n = self.partial_len.min(bytes.len());
                        self.writes.push(bytes[..n].to_vec());
                        self.partial_done = true;
                        return Ok(n);
                    }
                }
            }
            self.writes.push(bytes.to_vec());
            Ok(bytes.len())
        }
        fn read(&mut self, _buf: &mut [u8], _timeout_ms: u32) -> Result<usize, AdapterError> {
            Err(AdapterError::Unsupported("read"))
        }
        fn close(&mut self) {
            self.opened = false;
        }
    }

    fn matching_candidate() -> PortCandidate {
        PortCandidate {
            key: expected_instance_id(),
            description: expected_instance_id(),
            transport: TransportKind::Cdc,
            stable_id: Some(INSTANCE_SUFFIX.to_string()),
            open_path: Some(r"\\?\usbserial".to_string()),
        }
    }

    fn frame() -> Frame {
        Frame::new(
            DEVICE_WIDTH,
            DEVICE_HEIGHT,
            PixelFormat::Rgb565Le,
            vec![0u8; (DEVICE_WIDTH * DEVICE_HEIGHT * 2) as usize],
            Some(LEGACY_CHUNK_SIZE),
        )
        .expect("frame")
    }

    #[test]
    fn discover_accepts_only_exact_instance_id() {
        let mut adapter = TurzxCdcAdapter::new(MockTransport {
            candidates: vec![
                PortCandidate {
                    key: "ACPI\\PNP0501\\0".into(),
                    description: "COM1".into(),
                    transport: TransportKind::Cdc,
                    stable_id: Some("0".into()),
                    open_path: None,
                },
                matching_candidate(),
            ],
            ..Default::default()
        });
        let info = adapter.discover().expect("discover");
        assert_eq!(info.key, expected_instance_id());
    }

    #[test]
    fn discover_rejects_missing_and_multiple() {
        let mut adapter = TurzxCdcAdapter::new(MockTransport::default());
        assert_eq!(adapter.discover(), Err(DiscoveryError::NotFound));

        let mut adapter = TurzxCdcAdapter::new(MockTransport {
            candidates: vec![matching_candidate(), matching_candidate()],
            ..Default::default()
        });
        assert!(matches!(
            adapter.discover(),
            Err(DiscoveryError::Multiple(_))
        ));
    }

    #[test]
    fn discover_rejects_wrong_suffix_and_missing_path() {
        let mut candidate = matching_candidate();
        candidate.stable_id = Some("OTHER".into());
        let mut adapter = TurzxCdcAdapter::new(MockTransport {
            candidates: vec![candidate],
            ..Default::default()
        });
        assert!(matches!(
            adapter.discover(),
            Err(DiscoveryError::AmbiguousInstance(_))
        ));

        let mut candidate = matching_candidate();
        candidate.open_path = None;
        let mut adapter = TurzxCdcAdapter::new(MockTransport {
            candidates: vec![candidate],
            ..Default::default()
        });
        assert!(matches!(
            adapter.discover(),
            Err(DiscoveryError::NotOpenable(_))
        ));
    }

    #[test]
    fn init_sends_screen_on_and_brightness_zero_in_order() {
        let mut candidate = matching_candidate();
        candidate.open_path = Some("path".into());
        let mut adapter = TurzxCdcAdapter::new(MockTransport {
            candidates: vec![candidate],
            ..Default::default()
        })
        .with_init_delay_ms(0);
        let info = adapter.discover().expect("discover");
        adapter.open(&info).expect("open");
        adapter.init().expect("init");
        let t = adapter.transport();
        assert_eq!(t.writes.len(), 2);
        assert_eq!(t.writes[0], [0, 0, 0, 0, 0, 0x6D]);
        assert_eq!(t.writes[1], brightness_zero());
    }

    #[test]
    fn open_busy_is_propagated() {
        let mut adapter = TurzxCdcAdapter::new(MockTransport {
            open_fails_busy: true,
            ..Default::default()
        });
        let info = DeviceInfo {
            key: expected_instance_id(),
            model_id: MODEL_ID.into(),
            stable_id: Some(INSTANCE_SUFFIX.into()),
            transport: TransportKind::Cdc,
            open_path: Some("path".into()),
        };
        assert_eq!(adapter.open(&info), Err(AdapterError::Busy));
    }

    #[test]
    fn frame_is_header_plus_38_chunks() {
        let mut candidate = matching_candidate();
        candidate.open_path = Some("path".into());
        let mut adapter = TurzxCdcAdapter::new(MockTransport {
            candidates: vec![candidate],
            ..Default::default()
        })
        .with_init_delay_ms(0);
        let info = adapter.discover().expect("discover");
        adapter.open(&info).expect("open");
        adapter.init().expect("init");
        adapter.send_frame(&frame()).expect("send");

        let t = adapter.transport();
        assert_eq!(t.writes.len(), 2 + 1 + 38); // 初始化 2 条 + 帧头 + 38 块
        assert_eq!(t.writes[0], [0, 0, 0, 0, 0, 0x6D]);
        assert_eq!(t.writes[1], brightness_zero());
        assert_eq!(t.writes[2], [0, 0, 0x04, 0xFD, 0xDF, 0xC5]);
        assert_eq!(t.writes[3].len(), 8192);
        assert_eq!(t.writes.last().expect("last").len(), 4096);
        let report = adapter.last_report().expect("report");
        assert_eq!(report.bytes, 6 + 307_200);
        assert_eq!(report.chunks, 38);
        assert_eq!(report.last_chunk_len, 4096);
    }

    #[test]
    fn capabilities_are_320x480_portrait_without_device_rotation() {
        let adapter = TurzxCdcAdapter::new(MockTransport::default());
        let caps = adapter.capabilities();
        assert_eq!(caps.frame_width.known(), Some(&320));
        assert_eq!(caps.frame_height.known(), Some(&480));
        assert_eq!(
            caps.native_orientation.known(),
            Some(&Orientation::Portrait)
        );
        assert_eq!(caps.rotation_modes.known(), Some(&vec![Rotation::None]));
        assert_eq!(caps.pixel_format.known(), Some(&PixelFormat::Rgb565Le));
        assert_eq!(caps.chunk_size.known(), Some(&8192));
    }

    #[test]
    fn rejects_480x320_physical_frame() {
        let mut candidate = matching_candidate();
        candidate.open_path = Some("path".into());
        let mut adapter = TurzxCdcAdapter::new(MockTransport {
            candidates: vec![candidate],
            ..Default::default()
        })
        .with_init_delay_ms(0);
        let info = adapter.discover().expect("discover");
        adapter.open(&info).expect("open");
        adapter.init().expect("init");
        let bad = Frame::new(
            480,
            320,
            PixelFormat::Rgb565Le,
            vec![0u8; 480 * 320 * 2],
            Some(LEGACY_CHUNK_SIZE),
        )
        .expect("bad frame");
        assert_eq!(
            adapter.send_frame(&bad),
            Err(AdapterError::Unsupported("帧必须是 320×480 RGB565LE"))
        );
    }

    #[test]
    fn partial_write_is_completed_without_truncation() {
        let mut candidate = matching_candidate();
        candidate.open_path = Some("path".into());
        let mut adapter = TurzxCdcAdapter::new(MockTransport {
            candidates: vec![candidate],
            partial_write_at: Some(2),
            partial_len: 3,
            ..Default::default()
        })
        .with_init_delay_ms(0);
        let info = adapter.discover().expect("discover");
        adapter.open(&info).expect("open");
        adapter.init().expect("init");
        adapter.send_frame(&frame()).expect("send");

        let t = adapter.transport();
        assert_eq!(t.writes.len(), 2 + 2 + 38);
        assert_eq!(t.writes[2], [0, 0, 0x04]);
        assert_eq!(t.writes[3], [0xFD, 0xDF, 0xC5]);
        let header: Vec<u8> = t.writes[2]
            .iter()
            .chain(t.writes[3].iter())
            .copied()
            .collect();
        assert_eq!(header, vec![0, 0, 0x04, 0xFD, 0xDF, 0xC5]);
        let report = adapter.last_report().expect("report");
        assert_eq!(report.bytes, 6 + 307_200);
        assert_eq!(report.chunks, 38);
    }

    #[test]
    fn write_timeout_is_reported() {
        let mut candidate = matching_candidate();
        candidate.open_path = Some("path".into());
        let mut adapter = TurzxCdcAdapter::new(MockTransport {
            candidates: vec![candidate],
            fail_write_timeout_at: Some(2),
            ..Default::default()
        })
        .with_init_delay_ms(0);
        let info = adapter.discover().expect("discover");
        adapter.open(&info).expect("open");
        adapter.init().expect("init");
        assert_eq!(adapter.send_frame(&frame()), Err(AdapterError::Timeout));
    }

    #[test]
    fn frame_pixels_are_307200_bytes() {
        let frame = frame();
        assert_eq!((frame.width, frame.height), (320, 480));
        assert_eq!(frame.pixels.len(), 307_200);
        assert_eq!(
            frame_header(frame.width, frame.height).expect("header"),
            [0, 0, 0x04, 0xFD, 0xDF, 0xC5]
        );
    }

    #[test]
    fn write_failure_is_reported() {
        let mut candidate = matching_candidate();
        candidate.open_path = Some("path".into());
        let mut adapter = TurzxCdcAdapter::new(MockTransport {
            candidates: vec![candidate],
            fail_write_at: Some(0),
            ..Default::default()
        })
        .with_init_delay_ms(0);
        let info = adapter.discover().expect("discover");
        adapter.open(&info).expect("open");
        assert_eq!(adapter.init(), Err(AdapterError::Disconnected));
    }

    #[test]
    fn wrong_frame_and_brightness_are_rejected() {
        let mut adapter = TurzxCdcAdapter::new(MockTransport::default());
        let info = DeviceInfo {
            key: expected_instance_id(),
            model_id: MODEL_ID.into(),
            stable_id: None,
            transport: TransportKind::Cdc,
            open_path: None,
        };
        assert_eq!(
            adapter.send_frame(&frame()),
            Err(AdapterError::Disconnected)
        );
        assert_eq!(
            adapter.set_brightness(10),
            Err(AdapterError::Unsupported("亮度档位未验证，仅允许 0"))
        );
        assert_eq!(
            adapter.open(&info),
            Err(AdapterError::Unsupported("缺少接口路径"))
        );
        let _ = adapter.poll_health();
    }
}
