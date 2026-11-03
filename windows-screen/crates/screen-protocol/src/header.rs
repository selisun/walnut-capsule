//! 6 字节候选帧头（旧系统组合级真机基线，不是官方协议标准）。

use crate::error::ProtocolError;

pub const HEADER_LEN: usize = 6;
pub const END_MARKER: u8 = 0xC5;

/// 480×320 的候选测试向量（由公式推导，见文档 04 §4.3）。
pub const LEGACY_480X320_HEADER: [u8; HEADER_LEN] = [0x00, 0x00, 0x07, 0x7D, 0x3F, END_MARKER];

/// `0,0,(W-1)>>6,(((W-1)&63)<<2)|((H-1)>>8),(H-1)&255,0xC5`
pub fn frame_header(width: u32, height: u32) -> Result<[u8; HEADER_LEN], ProtocolError> {
    // 宽度用 6+8 位表示：W-1 <= 16383；高度用 8+2 位表示：H-1 <= 1023。
    if width == 0 || height == 0 || width > 16_384 || height > 1_024 {
        return Err(ProtocolError::InvalidDimensions { width, height });
    }
    let w = width - 1;
    let h = height - 1;
    Ok([
        0,
        0,
        ((w >> 6) & 0xFF) as u8,
        ((((w & 63) << 2) | (h >> 8)) & 0xFF) as u8,
        (h & 0xFF) as u8,
        END_MARKER,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_candidate_vector_for_480x320() {
        assert_eq!(frame_header(480, 320).unwrap(), LEGACY_480X320_HEADER);
    }

    #[test]
    fn header_is_computed_from_dimensions() {
        assert_eq!(
            frame_header(320, 480).unwrap(),
            [0, 0, 0x04, 0xFD, 0xDF, 0xC5]
        );
    }

    #[test]
    fn invalid_dimensions_are_rejected() {
        assert!(frame_header(0, 320).is_err());
        assert!(frame_header(480, 0).is_err());
        assert!(frame_header(16_385, 320).is_err());
        assert!(frame_header(480, 1_025).is_err());
    }
}
