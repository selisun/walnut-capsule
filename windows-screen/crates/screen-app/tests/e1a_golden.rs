// E1-A Golden 逐字节回归：320×480、RGB565LE、固定 307,206 B。
// 该测试不得修改 Golden；若生成算法变化，必须修生成器，不得改 Golden。

use screen_app::pattern;
use screen_core::{Frame, PixelFormat};
use screen_protocol::{frame_header, rgba_to_rgb565le};

const GOLDEN: &[u8] = include_bytes!("data/e1a_golden_320x480.rgb565");
const W: u32 = 320;
const H: u32 = 480;

fn set_pixel(buf: &mut [u8], x: u32, y: u32, c: [u8; 4]) {
    let i = ((y * W + x) * 4) as usize;
    buf[i..i + 4].copy_from_slice(&c);
}

fn fill_rect(buf: &mut [u8], x0: u32, y0: u32, ww: u32, hh: u32, c: [u8; 4]) {
    for y in y0..y0 + hh {
        for x in x0..x0 + ww {
            set_pixel(buf, x, y, c);
        }
    }
}

fn e1a_rgba() -> Vec<u8> {
    let mut px = vec![0u8; (W * H * 4) as usize];
    fill_rect(&mut px, 0, 0, W, H, [128, 128, 128, 255]);
    const BAR_COLORS: [[u8; 3]; 8] = [
        [255, 255, 255],
        [255, 255, 0],
        [0, 255, 255],
        [0, 255, 0],
        [255, 0, 255],
        [255, 0, 0],
        [0, 0, 255],
        [0, 0, 0],
    ];
    for (i, c) in BAR_COLORS.iter().enumerate() {
        fill_rect(
            &mut px,
            16 + i as u32 * 36,
            64,
            36,
            64,
            [c[0], c[1], c[2], 255],
        );
    }
    for (i, c) in BAR_COLORS.iter().enumerate() {
        fill_rect(
            &mut px,
            16,
            160 + i as u32 * 20,
            64,
            20,
            [c[0], c[1], c[2], 255],
        );
    }
    for y in 160..=239 {
        for x in 160..=303 {
            let v = ((x - 160) * 255 / 143) as u8;
            set_pixel(&mut px, x, y, [v, v, v, 255]);
        }
    }
    for y in 240..=319 {
        for x in 160..=303 {
            let v = ((y - 240) * 255 / 79) as u8;
            set_pixel(&mut px, x, y, [v, v, v, 255]);
        }
    }
    fill_rect(&mut px, 158, 200, 4, 80, [255, 255, 255, 255]);
    fill_rect(&mut px, 120, 238, 80, 4, [255, 255, 255, 255]);
    fill_rect(&mut px, 12, 12, 32, 32, [255, 0, 0, 255]);
    fill_rect(&mut px, W - 44, 12, 32, 32, [0, 255, 0, 255]);
    fill_rect(&mut px, 12, H - 44, 32, 32, [0, 0, 255, 255]);
    fill_rect(&mut px, W - 44, H - 44, 32, 32, [255, 255, 255, 255]);
    fill_rect(&mut px, 0, 0, W, 4, [255, 255, 0, 255]);
    fill_rect(&mut px, 0, H - 4, W, 4, [255, 255, 0, 255]);
    fill_rect(&mut px, 0, 0, 4, H, [255, 255, 0, 255]);
    fill_rect(&mut px, W - 4, 0, 4, H, [255, 255, 0, 255]);
    px
}

fn e1a_frame() -> Vec<u8> {
    let rgba = e1a_rgba();
    let pixels = rgba_to_rgb565le(&rgba, W, H).expect("rgb565");
    let header = frame_header(W, H).expect("header");
    assert_eq!(header, [0, 0, 0x04, 0xFD, 0xDF, 0xC5]);
    let mut frame = Vec::with_capacity(header.len() + pixels.len());
    frame.extend_from_slice(&header);
    frame.extend_from_slice(&pixels);
    frame
}

#[test]
fn e1a_golden_is_preserved_and_matches_byte_for_byte() {
    assert_eq!(GOLDEN.len(), 307_206);
    assert_eq!(&GOLDEN[..6], &[0, 0, 0x04, 0xFD, 0xDF, 0xC5]);

    let generated = e1a_frame();
    assert_eq!(generated.len(), GOLDEN.len());
    assert_eq!(
        &generated[..6],
        &GOLDEN[..6],
        "E1-A Golden 帧头必须完全一致"
    );
    // 像素数据逐字节比较，不用尺寸/字符串/SHA256 代替。
    assert_eq!(
        &generated[6..],
        &GOLDEN[6..],
        "E1-A Golden 像素数据必须逐字节一致"
    );
}

#[test]
fn production_pattern_is_320x480_rgb565le_and_not_golden_mutated() {
    let frame = pattern::build(pattern::Pattern::Solid).expect("solid");
    assert_eq!((frame.width, frame.height), (320, 480));
    assert_eq!(frame.pixels.len(), 307_200);
    assert_eq!(frame.pixel_format, PixelFormat::Rgb565Le);
    assert_eq!(pattern::WIDTH, 320);
    assert_eq!(pattern::HEIGHT, 480);
    assert_eq!(
        pattern::WIDTH,
        screen_adapter_turzx_cdc::DEVICE_WIDTH,
        "测试图案与适配器物理宽必须一致"
    );
    assert_eq!(
        pattern::HEIGHT,
        screen_adapter_turzx_cdc::DEVICE_HEIGHT,
        "测试图案与适配器物理高必须一致"
    );
}

#[test]
fn e1a_known_frame_shape_is_still_valid() {
    let frame = Frame::new(
        W,
        H,
        PixelFormat::Rgb565Le,
        e1a_frame()[6..].to_vec(),
        Some(8192),
    )
    .expect("frame");
    assert_eq!(frame.pixels.len(), 307_200);
}
