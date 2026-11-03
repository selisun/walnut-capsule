//! 帧缓冲与输出帧。转换（RGB565/旋转）在后续阶段实现。

use crate::capabilities::PixelFormat;
use crate::error::CoreError;

pub const RGBA_BYTES_PER_PIXEL: usize = 4;

/// 逻辑方向 RGBA8 缓冲，由 `screen-render` 产出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameBuffer {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl FrameBuffer {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, CoreError> {
        let expected = checked_len(width, height, RGBA_BYTES_PER_PIXEL)?;
        if pixels.len() != expected {
            return Err(CoreError::BufferLength {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn zeroed(width: u32, height: u32) -> Result<Self, CoreError> {
        let len = checked_len(width, height, RGBA_BYTES_PER_PIXEL)?;
        Self::new(width, height, vec![0; len])
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    /// 转为可下发的帧（当前仍为 RGBA8；设备格式转换在 `screen-protocol`）。
    pub fn into_frame(self, chunk_hint: Option<usize>) -> Frame {
        Frame {
            width: self.width,
            height: self.height,
            pixel_format: PixelFormat::Rgba8,
            pixels: self.pixels,
            chunk_hint,
        }
    }
}

/// 待下发字节帧：只描述尺寸/格式/字节，不含传输方式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    pub pixels: Vec<u8>,
    pub chunk_hint: Option<usize>,
}

impl Frame {
    pub fn new(
        width: u32,
        height: u32,
        pixel_format: PixelFormat,
        pixels: Vec<u8>,
        chunk_hint: Option<usize>,
    ) -> Result<Self, CoreError> {
        let bpp = bytes_per_pixel(pixel_format);
        let expected = checked_len(width, height, bpp)?;
        if pixels.len() != expected {
            return Err(CoreError::BufferLength {
                expected,
                actual: pixels.len(),
            });
        }
        if chunk_hint == Some(0) {
            return Err(CoreError::InvalidArgument("chunk_hint"));
        }
        Ok(Self {
            width,
            height,
            pixel_format,
            pixels,
            chunk_hint,
        })
    }
}

pub fn bytes_per_pixel(format: PixelFormat) -> usize {
    match format {
        PixelFormat::Rgb565Le | PixelFormat::Rgb565Be => 2,
        PixelFormat::Rgb888 => 3,
        PixelFormat::Mono => 1,
        PixelFormat::Rgba8 => 4,
    }
}

fn checked_len(width: u32, height: u32, bpp: usize) -> Result<usize, CoreError> {
    let pixels =
        (width as usize)
            .checked_mul(height as usize)
            .ok_or(CoreError::NumericOverflow {
                field: "width*height",
            })?;
    pixels.checked_mul(bpp).ok_or(CoreError::NumericOverflow {
        field: "buffer_bytes",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_buffer_length_is_enforced() {
        assert!(FrameBuffer::new(2, 2, vec![0; 16]).is_ok());
        assert!(FrameBuffer::new(2, 2, vec![0; 15]).is_err());
    }

    #[test]
    fn frame_length_per_format() {
        let frame = Frame::new(2, 2, PixelFormat::Rgb565Le, vec![0; 8], Some(8)).expect("frame");
        assert_eq!(frame.pixels.len(), 8);
        assert!(Frame::new(2, 2, PixelFormat::Rgb888, vec![0; 11], None).is_err());
    }
}
