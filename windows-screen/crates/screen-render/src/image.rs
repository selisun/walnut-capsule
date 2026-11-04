//! 主题背景图。仅解码 PNG（旧版主题素材均为 PNG），不引入新 crate。

use tiny_skia::Pixmap;

use crate::error::RenderError;

/// 已解码的背景图。尺寸由主题 `background.fit/anchor` 在绘制时决定。
pub struct BackgroundImage {
    pixmap: Pixmap,
}

impl BackgroundImage {
    pub fn from_png(bytes: &[u8]) -> Result<Self, RenderError> {
        let pixmap = Pixmap::decode_png(bytes)
            .map_err(|e| RenderError::Canvas(format!("背景图解码失败：{e}")))?;
        if pixmap.width() == 0 || pixmap.height() == 0 {
            return Err(RenderError::Canvas("背景图尺寸为空".to_string()));
        }
        Ok(Self { pixmap })
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    pub(crate) fn pixmap(&self) -> &Pixmap {
        &self.pixmap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_bytes_are_rejected() {
        assert!(BackgroundImage::from_png(&[0, 1, 2, 3]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn png_roundtrip_works() {
        let mut pixmap = Pixmap::new(4, 3).expect("pixmap");
        pixmap.fill(tiny_skia::Color::from_rgba8(10, 20, 30, 255));
        let png = pixmap.encode_png().expect("encode");
        let image = BackgroundImage::from_png(&png).expect("decode");
        assert_eq!(image.width(), 4);
        assert_eq!(image.height(), 3);
    }
}
