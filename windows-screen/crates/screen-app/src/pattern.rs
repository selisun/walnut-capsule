//! 3.5 英寸测试图案（320×480 物理基线）。
//!
//! E1-A 真机验证的物理帧为 320×480；本模块直接生成 320×480 RGB565LE，
//! 不再旋转成 480×320。横屏逻辑图案由测试专用 fixture 生成，不进入物理输出。
//! 注意：这是测试图案链路，不是最终渲染管线；不改变任何协议参数。

use screen_core::{Frame, PixelFormat};
use screen_protocol::rgba_to_rgb565le;

pub const WIDTH: u32 = 320;
pub const HEIGHT: u32 = 480;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    Bars,
    Gradient,
    Solid,
}

impl Pattern {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "bars" => Some(Self::Bars),
            "gradient" => Some(Self::Gradient),
            "solid" => Some(Self::Solid),
            _ => None,
        }
    }
}

pub fn build(pattern: Pattern) -> Result<Frame, String> {
    let portrait = match pattern {
        Pattern::Bars => bars(),
        Pattern::Gradient => gradient(),
        Pattern::Solid => solid(),
    };
    let pixels = rgba_to_rgb565le(&portrait, WIDTH, HEIGHT).map_err(|e| e.to_string())?;
    Frame::new(WIDTH, HEIGHT, PixelFormat::Rgb565Le, pixels, Some(8192)).map_err(|e| e.to_string())
}

/// 旧系统 `RotateFlip(Rotate90FlipNone)` 的逐像素等价：dst(x,y) = src(y, H-1-x)。
/// 仅保留给历史 480×320 审计；生产 `build` 不再调用。
pub fn rotate_cw(src: &[u8], src_w: u32, src_h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    for y in 0..src_w {
        for x in 0..src_h {
            let sx = y;
            let sy = src_h - 1 - x;
            let i = ((sy * src_w + sx) as usize) * 4;
            out.extend_from_slice(&src[i..i + 4]);
        }
    }
    out
}

fn bars() -> Vec<u8> {
    const COLORS: [[u8; 3]; 8] = [
        [255, 255, 255],
        [255, 255, 0],
        [0, 255, 255],
        [0, 255, 0],
        [255, 0, 255],
        [255, 0, 0],
        [0, 0, 255],
        [0, 0, 0],
    ];
    let mut out = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for _y in 0..HEIGHT {
        for x in 0..WIDTH {
            let band = (x * 8 / WIDTH) as usize;
            let c = COLORS[band.min(7)];
            out.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    out
}

fn gradient() -> Vec<u8> {
    let mut out = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for _y in 0..HEIGHT {
        for x in 0..WIDTH {
            let v = (x * 255 / (WIDTH - 1)) as u8;
            out.extend_from_slice(&[v, v, v, 255]);
        }
    }
    out
}

fn solid() -> Vec<u8> {
    let mut out = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for _ in 0..(WIDTH * HEIGHT) {
        out.extend_from_slice(&[128, 128, 128, 255]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use screen_core::{bytes_per_pixel, FrameBuffer};
    use screen_render::rotate90;

    fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 2] {
        let offset = ((y * WIDTH + x) * bytes_per_pixel(PixelFormat::Rgb565Le) as u32) as usize;
        [frame.pixels[offset], frame.pixels[offset + 1]]
    }

    /// 320×480 全尺寸唯一像素：R=x 低 8 位、G=y 低 8 位、B=(x>>8)&1 | ((y>>8)&1)<<1。
    fn unique_source() -> Vec<u8> {
        let mut src = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                src.extend_from_slice(&[
                    (x & 0xFF) as u8,
                    (y & 0xFF) as u8,
                    ((((x >> 8) & 1) | (((y >> 8) & 1) << 1)) & 0xFF) as u8,
                    255,
                ]);
            }
        }
        src
    }

    #[test]
    fn frame_is_320x480_rgb565le() {
        let frame = build(Pattern::Solid).expect("solid");
        assert_eq!((frame.width, frame.height), (320, 480));
        assert_eq!(frame.pixels.len(), 307_200);
        assert_eq!(frame.pixel_format, PixelFormat::Rgb565Le);
    }

    #[test]
    fn bars_are_vertical_in_portrait() {
        let frame = build(Pattern::Bars).expect("bars");
        let expected = [
            [0xFF, 0xFF],
            [0xE0, 0xFF],
            [0xFF, 0x07],
            [0xE0, 0x07],
            [0x1F, 0xF8],
            [0x00, 0xF8],
            [0x1F, 0x00],
            [0x00, 0x00],
        ];
        for (band, bytes) in expected.iter().enumerate() {
            let x = band as u32 * 40 + 20;
            assert_eq!(pixel(&frame, x, 10), *bytes, "band {band} x={x}");
        }
    }

    #[test]
    fn bars_are_same_color_down_column_and_end_black() {
        let frame = build(Pattern::Bars).expect("bars");
        for x in [0, 159, 319] {
            assert_eq!(pixel(&frame, x, 0), pixel(&frame, x, 479));
        }
        assert_eq!(pixel(&frame, 319, 0), [0x00, 0x00]);
    }

    #[test]
    fn gradient_is_horizontal_in_portrait() {
        let frame = build(Pattern::Gradient).expect("gradient");
        assert_eq!(pixel(&frame, 0, 0), [0x00, 0x00]);
        let (lo, hi) = (pixel(&frame, 319, 0)[0], pixel(&frame, 319, 0)[1]);
        assert!(
            lo > 0xE0 && hi > 0xE0,
            "right edge should be near white: {lo:02X} {hi:02X}"
        );
        assert_eq!(pixel(&frame, 0, 0), pixel(&frame, 0, 479));
    }

    #[test]
    fn rotate_cw_full_size_maps_all_153600_pixels_for_legacy_audit() {
        const SRC_W: u32 = 320;
        const SRC_H: u32 = 480;
        const DST_W: u32 = 480;
        const DST_H: u32 = 320;
        let src = unique_source();
        assert_eq!(src.len(), (SRC_W * SRC_H * 4) as usize);
        let out = rotate_cw(&src, SRC_W, SRC_H);
        assert_eq!(out.len(), (DST_W * DST_H * 4) as usize);

        for dy in 0..DST_H {
            for dx in 0..DST_W {
                let sx = dy;
                let sy = SRC_H - 1 - dx;
                let si = ((sy * SRC_W + sx) * 4) as usize;
                let di = ((dy * DST_W + dx) * 4) as usize;
                assert_eq!(
                    &out[di..di + 4],
                    &src[si..si + 4],
                    "dst({dx},{dy}) 应来自 src({sx},{sy})"
                );
            }
        }
    }

    #[test]
    fn rotate_cw_matches_screen_render_rotate90_for_legacy_audit() {
        const SRC_W: u32 = 320;
        const SRC_H: u32 = 480;
        let src = unique_source();
        let out = rotate_cw(&src, SRC_W, SRC_H);
        let buffer = FrameBuffer::new(SRC_W, SRC_H, src).expect("source buffer");
        let rendered = rotate90(&buffer);
        assert_eq!((rendered.width(), rendered.height()), (480, 320));
        assert_eq!(rendered.pixels(), out.as_slice());
    }
}
