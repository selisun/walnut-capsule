//! 帧间差分：决定下一拍发全帧、只发脏矩形，还是完全不发。
//!
//! 设备无双缓冲，每发一次整帧都会把 320×480 逐段重写；内容不变时重复发整帧
//! 就是持续闪屏。只在内容变化时发、且只发变化区域，可消除这种重画。

use screen_adapter_api::Region;
use screen_core::{bytes_per_pixel, Frame};

/// 脏区面积达到整屏这个比例时改发全帧：此时区域帧的传输量与全帧已无差别，
/// 而全帧是官方实际使用、真机验证过的路径，保留作为兜底。
pub const FULL_FRAME_RATIO_PERCENT: u64 = 60;

/// 脏矩形的最小边长。
///
/// 1×1、奇数宽度、单行这类尺寸尚未在任何真机上验证过设备行为；先把矩形
/// 外扩到 2×2 以上，用几个像素的传输量换掉「尺寸边界未知」的风险。
/// 真机确认 1×1 可用后可以下调为 1。
pub const MIN_REGION_SIZE: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSend {
    /// 首帧、尺寸/方向变化、或脏区过大。
    Full,
    /// 只有一块区域变化。
    Region(Region),
    /// 与上一帧完全相同，不发。
    Skip,
}

/// 与上一帧相比本拍应如何发送；`None` 表示还没有上一帧（发全帧）。
pub fn plan_send(previous: Option<&Frame>, current: &Frame) -> FrameSend {
    let Some(previous) = previous else {
        return FrameSend::Full;
    };
    if previous.width != current.width
        || previous.height != current.height
        || previous.pixel_format != current.pixel_format
    {
        return FrameSend::Full;
    }
    let Some(region) = changed_region(previous, current) else {
        return FrameSend::Skip;
    };
    let full_area = u64::from(current.width) * u64::from(current.height);
    if full_area == 0 || region.area() * 100 >= full_area * FULL_FRAME_RATIO_PERCENT {
        FrameSend::Full
    } else {
        FrameSend::Region(region)
    }
}

/// 两帧不同像素的最小包围盒；完全相同返回 `None`。
///
/// 尺寸/格式不一致时返回整帧区域（调用方应直接走全帧）。
pub fn changed_region(previous: &Frame, current: &Frame) -> Option<Region> {
    let full = || Region::new(0, 0, current.width, current.height);
    if previous.width != current.width
        || previous.height != current.height
        || previous.pixel_format != current.pixel_format
        || previous.pixels.len() != current.pixels.len()
    {
        return Some(full());
    }
    let bpp = bytes_per_pixel(current.pixel_format);
    let row_len = current.width as usize * bpp;
    if bpp == 0 || row_len == 0 {
        return Some(full());
    }

    let mut min_x = current.width;
    let mut max_x = 0u32;
    let mut min_y = current.height;
    let mut max_y = 0u32;
    for row in 0..current.height as usize {
        let start = row * row_len;
        let prev_row = &previous.pixels[start..start + row_len];
        let cur_row = &current.pixels[start..start + row_len];
        if prev_row == cur_row {
            continue;
        }
        let first = prev_row
            .iter()
            .zip(cur_row)
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        let last = prev_row
            .iter()
            .zip(cur_row)
            .rposition(|(a, b)| a != b)
            .unwrap_or(row_len - 1);
        let x0 = (first / bpp) as u32;
        let x1 = (last / bpp) as u32;
        if x0 < min_x {
            min_x = x0;
        }
        if x1 > max_x {
            max_x = x1;
        }
        let y = row as u32;
        if y < min_y {
            min_y = y;
        }
        if y > max_y {
            max_y = y;
        }
    }
    if max_y < min_y || max_x < min_x {
        return None;
    }
    Some(expand_to_min_size(
        min_x,
        min_y,
        max_x - min_x + 1,
        max_y - min_y + 1,
        current.width,
        current.height,
    ))
}

/// 把矩形外扩到至少 `MIN_REGION_SIZE` 见方，并保证不越过帧边界。
fn expand_to_min_size(x: u32, y: u32, w: u32, h: u32, frame_w: u32, frame_h: u32) -> Region {
    let w = w.max(MIN_REGION_SIZE).min(frame_w);
    let h = h.max(MIN_REGION_SIZE).min(frame_h);
    Region::new(
        x.min(frame_w.saturating_sub(w)),
        y.min(frame_h.saturating_sub(h)),
        w,
        h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use screen_core::{Frame, PixelFormat};

    const W: u32 = 320;
    const H: u32 = 480;

    fn frame(fill: u8) -> Frame {
        Frame::new(
            W,
            H,
            PixelFormat::Rgb565Le,
            vec![fill; (W * H * 2) as usize],
            Some(8192),
        )
        .expect("frame")
    }

    fn pixel_offset(x: u32, y: u32) -> usize {
        ((y * W + x) * 2) as usize
    }

    #[test]
    fn no_previous_frame_sends_full() {
        assert_eq!(plan_send(None, &frame(0)), FrameSend::Full);
    }

    #[test]
    fn identical_frames_are_skipped() {
        let prev = frame(7);
        assert_eq!(changed_region(&prev, &prev), None);
        assert_eq!(plan_send(Some(&prev), &frame(7)), FrameSend::Skip);
    }

    #[test]
    fn single_pixel_change_is_expanded_to_min_region() {
        let prev = frame(0);
        let mut cur = frame(0);
        let at = pixel_offset(20, 200);
        cur.pixels[at] = 0xFF;
        // 1×1 未在真机验证过，按 MIN_REGION_SIZE 外扩为 2×2
        let expected = Region::new(20, 200, 2, 2);
        assert_eq!(changed_region(&prev, &cur), Some(expected));
        assert_eq!(plan_send(Some(&prev), &cur), FrameSend::Region(expected));
    }

    #[test]
    fn min_region_expansion_stays_inside_frame() {
        let prev = frame(0);

        // 左上角单像素：原位外扩
        let mut cur = frame(0);
        cur.pixels[pixel_offset(0, 0)] = 0xBB;
        assert_eq!(changed_region(&prev, &cur), Some(Region::new(0, 0, 2, 2)));

        // 右下角单像素：必须内缩，不能越界
        let mut cur = frame(0);
        cur.pixels[pixel_offset(W - 1, H - 1)] = 0xAA;
        let region = changed_region(&prev, &cur).expect("region");
        assert_eq!(region, Region::new(W - 2, H - 2, 2, 2));
        assert!(region.right() < W);
        assert!(region.bottom() < H);

        // 右边界单列：向左内缩到 2 列宽
        let mut cur = frame(0);
        cur.pixels[pixel_offset(W - 1, 100)] = 0xCC;
        let region = changed_region(&prev, &cur).expect("region");
        assert_eq!(region, Region::new(W - 2, 100, 2, 2));
        assert!(region.right() < W);
    }

    #[test]
    fn bounding_box_covers_far_corners() {
        let prev = frame(0);
        let mut cur = frame(0);
        cur.pixels[pixel_offset(5, 10)] = 0x11;
        cur.pixels[pixel_offset(300, 470)] = 0x22;
        assert_eq!(
            changed_region(&prev, &cur),
            Some(Region::new(5, 10, 296, 461))
        );
        // 296×461 / (320×480) = 88.9% → 超过阈值，回退全帧
        assert_eq!(plan_send(Some(&prev), &cur), FrameSend::Full);
    }

    #[test]
    fn region_below_threshold_is_sent_as_region() {
        let prev = frame(0);
        let mut cur = frame(0);
        for y in 100..140u32 {
            for x in 0..W {
                let at = pixel_offset(x, y);
                cur.pixels[at] = 0x33;
            }
        }
        // 320×40 = 12,800 / 153,600 = 8.3%
        assert_eq!(
            plan_send(Some(&prev), &cur),
            FrameSend::Region(Region::new(0, 100, 320, 40))
        );
    }

    #[test]
    fn region_above_threshold_becomes_full_frame() {
        let prev = frame(0);
        let mut cur = frame(0);
        for y in 0..300u32 {
            for x in 0..W {
                let at = pixel_offset(x, y);
                cur.pixels[at] = 0x44;
            }
        }
        // 320×300 = 62.5% → 全帧
        assert_eq!(plan_send(Some(&prev), &cur), FrameSend::Full);
    }

    #[test]
    fn size_change_forces_full_frame() {
        let prev = Frame::new(
            160,
            240,
            PixelFormat::Rgb565Le,
            vec![0; 160 * 240 * 2],
            Some(8192),
        )
        .expect("frame");
        let cur = frame(0);
        assert_eq!(changed_region(&prev, &cur), Some(Region::new(0, 0, W, H)));
        assert_eq!(plan_send(Some(&prev), &cur), FrameSend::Full);
    }
}
