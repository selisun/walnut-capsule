//! RGBA8 → RGB565 小端转换。低字节先写、行优先、无 padding。

use crate::error::ProtocolError;

pub const BYTES_PER_PIXEL: usize = 2;

pub fn rgba_to_rgb565le(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ProtocolError> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(ProtocolError::InvalidDimensions { width, height })?;
    let expected = pixels
        .checked_mul(4)
        .ok_or(ProtocolError::InvalidDimensions { width, height })?;
    if rgba.len() != expected {
        return Err(ProtocolError::BufferLength {
            expected,
            actual: rgba.len(),
        });
    }
    let mut out = Vec::with_capacity(pixels * BYTES_PER_PIXEL);
    for chunk in rgba.chunks_exact(4) {
        let r = chunk[0] as u16;
        let g = chunk[1] as u16;
        let b = chunk[2] as u16;
        let v = ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3);
        out.push((v & 0xFF) as u8);
        out.push((v >> 8) as u8);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_colors_are_encoded_little_endian() {
        // 红：R=255,G=0,B=0 → 0xF800 → 低字节 0x00，高字节 0xF8
        let red = rgba_to_rgb565le(&[255, 0, 0, 255], 1, 1).unwrap();
        assert_eq!(red, vec![0x00, 0xF8]);

        // 绿：0x07E0
        let green = rgba_to_rgb565le(&[0, 255, 0, 255], 1, 1).unwrap();
        assert_eq!(green, vec![0xE0, 0x07]);

        // 蓝：0x001F
        let blue = rgba_to_rgb565le(&[0, 0, 255, 255], 1, 1).unwrap();
        assert_eq!(blue, vec![0x1F, 0x00]);

        // 白：0xFFFF；黑：0x0000
        assert_eq!(
            rgba_to_rgb565le(&[255, 255, 255, 255], 1, 1).unwrap(),
            vec![0xFF, 0xFF]
        );
        assert_eq!(
            rgba_to_rgb565le(&[0, 0, 0, 255], 1, 1).unwrap(),
            vec![0x00, 0x00]
        );
    }

    #[test]
    fn output_length_is_checked() {
        assert_eq!(
            rgba_to_rgb565le(&[0; 7], 1, 2),
            Err(ProtocolError::BufferLength {
                expected: 8,
                actual: 7
            })
        );
        let out = rgba_to_rgb565le(&[0; 480 * 320 * 4], 480, 320).unwrap();
        assert_eq!(out.len(), 480 * 320 * 2);
    }
}
