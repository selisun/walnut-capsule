//! 帧变换：旋转、镜像、反色。设备方向命令 `0x79` 被禁用，方向在软件侧处理。

use screen_core::{FrameBuffer, Rotation};

pub fn rotate90(src: &FrameBuffer) -> FrameBuffer {
    let (w, h) = (src.width(), src.height());
    let mut out = vec![0u8; src.pixels().len()];
    for y in 0..h {
        for x in 0..w {
            let src_i = ((y * w + x) * 4) as usize;
            // 顺时针 90°：新坐标 (h - 1 - y, x)
            let nx = h - 1 - y;
            let ny = x;
            let dst_i = ((ny * h + nx) * 4) as usize;
            out[dst_i..dst_i + 4].copy_from_slice(&src.pixels()[src_i..src_i + 4]);
        }
    }
    FrameBuffer::new(h, w, out).expect("旋转后尺寸一致")
}

pub fn rotate180(src: &FrameBuffer) -> FrameBuffer {
    let (w, h) = (src.width(), src.height());
    let mut out = vec![0u8; src.pixels().len()];
    for y in 0..h {
        for x in 0..w {
            let src_i = ((y * w + x) * 4) as usize;
            let nx = w - 1 - x;
            let ny = h - 1 - y;
            let dst_i = ((ny * w + nx) * 4) as usize;
            out[dst_i..dst_i + 4].copy_from_slice(&src.pixels()[src_i..src_i + 4]);
        }
    }
    FrameBuffer::new(w, h, out).expect("旋转后尺寸一致")
}

pub fn rotate270(src: &FrameBuffer) -> FrameBuffer {
    let (w, h) = (src.width(), src.height());
    let mut out = vec![0u8; src.pixels().len()];
    for y in 0..h {
        for x in 0..w {
            let src_i = ((y * w + x) * 4) as usize;
            // 顺时针 270°（逆时针 90°）：源 (x,y) -> 目标 (y, w-1-x)
            let nx = y;
            let ny = w - 1 - x;
            let dst_i = ((ny * h + nx) * 4) as usize;
            out[dst_i..dst_i + 4].copy_from_slice(&src.pixels()[src_i..src_i + 4]);
        }
    }
    FrameBuffer::new(h, w, out).expect("旋转后尺寸一致")
}

/// 统一方向映射：0°/90°/180°/270° 全部在软件像素层完成。
pub fn orient(src: &FrameBuffer, rotation: Rotation) -> FrameBuffer {
    match rotation {
        Rotation::None => src.clone(),
        Rotation::Deg90 => rotate90(src),
        Rotation::Deg180 => rotate180(src),
        Rotation::Deg270 => rotate270(src),
    }
}

pub fn mirror_horizontal(src: &FrameBuffer) -> FrameBuffer {
    let (w, h) = (src.width(), src.height());
    let mut out = vec![0u8; src.pixels().len()];
    for y in 0..h {
        for x in 0..w {
            let src_i = ((y * w + x) * 4) as usize;
            let dst_i = ((y * w + (w - 1 - x)) * 4) as usize;
            out[dst_i..dst_i + 4].copy_from_slice(&src.pixels()[src_i..src_i + 4]);
        }
    }
    FrameBuffer::new(w, h, out).expect("镜像后尺寸一致")
}

pub fn invert(src: &FrameBuffer) -> FrameBuffer {
    let mut out = src.pixels().to_vec();
    for px in out.chunks_exact_mut(4) {
        px[0] = 255 - px[0];
        px[1] = 255 - px[1];
        px[2] = 255 - px[2];
        // alpha 保持不变
    }
    FrameBuffer::new(src.width(), src.height(), out).expect("反色后尺寸一致")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(frame: &FrameBuffer, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y * frame.width() + x) * 4) as usize;
        [
            frame.pixels()[idx],
            frame.pixels()[idx + 1],
            frame.pixels()[idx + 2],
            frame.pixels()[idx + 3],
        ]
    }

    fn sample() -> FrameBuffer {
        // 2×2，四个像素分别不同
        let mut px = Vec::new();
        for v in [10u8, 20, 30, 40] {
            px.extend_from_slice(&[v, v, v, 255]);
        }
        FrameBuffer::new(2, 2, px).expect("buffer")
    }

    fn unique_frame(w: u32, h: u32) -> FrameBuffer {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&[
                    (x & 0xFF) as u8,
                    (y & 0xFF) as u8,
                    ((((x >> 8) & 1) | (((y >> 8) & 1) << 1)) & 0xFF) as u8,
                    255,
                ]);
            }
        }
        FrameBuffer::new(w, h, px).expect("buffer")
    }

    #[test]
    fn rotate90_swaps_dimensions_and_pixels() {
        let src = sample();
        let out = rotate90(&src);
        assert_eq!(out.width(), 2);
        assert_eq!(out.height(), 2);
        // (0,0) -> (1,0)
        assert_eq!(pixel(&out, 1, 0), pixel(&src, 0, 0));
        // (0,1) -> (0,0)
        assert_eq!(pixel(&out, 0, 0), pixel(&src, 0, 1));
    }

    #[test]
    fn rotate90_full_480x320_to_320x480_maps_all_pixels() {
        let src = unique_frame(480, 320);
        let out = rotate90(&src);
        assert_eq!((out.width(), out.height()), (320, 480));
        for y in 0..480 {
            for x in 0..320 {
                assert_eq!(pixel(&out, x, y), pixel(&src, y, 319 - x));
            }
        }
    }

    #[test]
    fn rotate180_full_320x480_maps_all_pixels_without_swap() {
        let src = unique_frame(320, 480);
        let out = rotate180(&src);
        assert_eq!((out.width(), out.height()), (320, 480));
        for y in 0..480 {
            for x in 0..320 {
                assert_eq!(pixel(&out, x, y), pixel(&src, 319 - x, 479 - y));
            }
        }
    }

    #[test]
    fn rotate270_full_480x320_to_320x480_maps_all_pixels() {
        let src = unique_frame(480, 320);
        let out = rotate270(&src);
        assert_eq!((out.width(), out.height()), (320, 480));
        for y in 0..480 {
            for x in 0..320 {
                assert_eq!(pixel(&out, x, y), pixel(&src, 479 - y, x));
            }
        }
    }

    #[test]
    fn orient_covers_all_rotations_and_keeps_dimensions() {
        let src = unique_frame(320, 480);
        let none = orient(&src, Rotation::None);
        assert_eq!((none.width(), none.height()), (320, 480));
        assert_eq!(none.pixels(), src.pixels());
        let deg180 = orient(&src, Rotation::Deg180);
        assert_eq!((deg180.width(), deg180.height()), (320, 480));
        assert_eq!(deg180, rotate180(&src));
    }

    #[test]
    fn mirror_moves_left_to_right() {
        let src = sample();
        let out = mirror_horizontal(&src);
        assert_eq!(pixel(&out, 1, 0), pixel(&src, 0, 0));
        assert_eq!(pixel(&out, 0, 0), pixel(&src, 1, 0));
    }

    #[test]
    fn invert_keeps_alpha() {
        let src = sample();
        let out = invert(&src);
        assert_eq!(pixel(&out, 0, 0), [245, 245, 245, 255]);
    }
}
