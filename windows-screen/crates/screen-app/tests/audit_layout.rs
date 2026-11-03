//! 离线审计：全尺寸图案 → 旋转 → RGB565 → 帧头/分块，输出可逐字节比较的二进制。
//! 不打开设备、不发送任何帧。

use std::fs;
use std::path::PathBuf;

use screen_app::pattern::{build, Pattern, HEIGHT, WIDTH};
use screen_protocol::{chunk_ranges, frame_header, rgba_to_rgb565le, LEGACY_CHUNK_SIZE};

const PORTRAIT_WIDTH: u32 = 320;
const PORTRAIT_HEIGHT: u32 = 480;
const NAMES: [&str; 8] = [
    "white",
    "black",
    "gray",
    "rgb_blocks",
    "gradient_x",
    "gradient_y",
    "corners",
    "bars",
];

fn source_rgba(name: &str) -> Vec<u8> {
    const BARS: [[u8; 3]; 8] = [
        [255, 255, 255],
        [255, 255, 0],
        [0, 255, 255],
        [0, 255, 0],
        [255, 0, 255],
        [255, 0, 0],
        [0, 0, 255],
        [0, 0, 0],
    ];
    let mut out = Vec::with_capacity((PORTRAIT_WIDTH * PORTRAIT_HEIGHT * 4) as usize);
    for y in 0..PORTRAIT_HEIGHT {
        for x in 0..PORTRAIT_WIDTH {
            let (r, g, b) = match name {
                "white" => (255, 255, 255),
                "black" => (0, 0, 0),
                "gray" => (128, 128, 128),
                "bars" => {
                    let band = ((x * 8) / PORTRAIT_WIDTH).min(7) as usize;
                    let c = BARS[band];
                    (c[0], c[1], c[2])
                }
                "rgb_blocks" => {
                    if y < 160 {
                        (255, 0, 0)
                    } else if y < 320 {
                        (0, 255, 0)
                    } else {
                        (0, 0, 255)
                    }
                }
                "gradient_x" => {
                    let v = ((x * 255) / (PORTRAIT_WIDTH - 1)) as u8;
                    (v, v, v)
                }
                "gradient_y" => {
                    let v = ((y * 255) / (PORTRAIT_HEIGHT - 1)) as u8;
                    (v, v, v)
                }
                "corners" => {
                    if x < 64 && y < 64 {
                        (255, 0, 0)
                    } else if x >= PORTRAIT_WIDTH - 64 && y < 64 {
                        (0, 255, 0)
                    } else if x < 64 && y >= PORTRAIT_HEIGHT - 64 {
                        (0, 0, 255)
                    } else if x >= PORTRAIT_WIDTH - 64 && y >= PORTRAIT_HEIGHT - 64 {
                        (255, 255, 255)
                    } else {
                        (0, 0, 0)
                    }
                }
                other => panic!("未知审计图案：{other}"),
            };
            out.extend_from_slice(&[r, g, b, 255]);
        }
    }
    out
}

#[test]
fn dump_full_frame_for_offline_comparison() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("audit_rust");
    fs::create_dir_all(&dir).expect("创建审计输出目录");

    for name in NAMES {
        let source = source_rgba(name);
        assert_eq!(
            source.len(),
            (PORTRAIT_WIDTH * PORTRAIT_HEIGHT * 4) as usize,
            "{name} 源 RGBA 长度"
        );

        let rotated = source.clone();
        assert_eq!(
            rotated.len(),
            (WIDTH * HEIGHT * 4) as usize,
            "{name} RGBA 长度"
        );
        fs::write(dir.join(format!("{name}.rgba.bin")), &rotated).expect("写 RGBA");

        let rgb565 = rgba_to_rgb565le(&rotated, WIDTH, HEIGHT).expect("RGB565 转换");
        assert_eq!(rgb565.len(), 307_200, "{name} RGB565 长度");

        // 生产测试图案链路（P-1/P-2/P-3 实际使用的 build）与审计输出逐字节一致。
        let production = match name {
            "bars" => Some(Pattern::Bars),
            "gradient_x" => Some(Pattern::Gradient),
            "gray" => Some(Pattern::Solid),
            _ => None,
        };
        if let Some(pattern) = production {
            let frame = build(pattern).expect("生产测试图案");
            assert_eq!((frame.width, frame.height), (WIDTH, HEIGHT));
            assert_eq!(frame.pixels, rgb565, "{name} 与生产 build 输出不一致");
        }

        fs::write(dir.join(format!("{name}.rgb565.bin")), &rgb565).expect("写 RGB565");

        let header = frame_header(WIDTH, HEIGHT).expect("帧头");
        let ranges = chunk_ranges(rgb565.len(), LEGACY_CHUNK_SIZE).expect("分块");
        assert_eq!(ranges.len(), 38, "{name} 分块数");

        let mut reassembled = Vec::with_capacity(rgb565.len());
        for range in &ranges {
            reassembled.extend_from_slice(&rgb565[range.clone()]);
        }
        assert_eq!(reassembled, rgb565, "{name} 分块拼接顺序");

        let mut frame = Vec::with_capacity(header.len() + rgb565.len());
        frame.extend_from_slice(&header);
        frame.extend_from_slice(&rgb565);
        assert_eq!(frame.len(), 307_206, "{name} 完整帧长度");
        fs::write(dir.join(format!("{name}.frame.bin")), &frame).expect("写完整帧");

        println!(
            "audit={name} rgba={} rgb565={} frame={} header={:02X?} first={:02X?} last={:02X?}",
            rotated.len(),
            rgb565.len(),
            frame.len(),
            header,
            rgb565.first(),
            rgb565.last()
        );
    }

    println!("audit_rust_dir={}", dir.display());
}
