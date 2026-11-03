//! GDI 字形轮廓解析与缓存。仅 Windows 编译；不引入字体解析 crate。

#[cfg(windows)]
use std::collections::HashMap;

#[cfg(windows)]
use tiny_skia::{Path, PathBuilder};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    GLYPHMETRICS, MAT2, POINTFX, TTPOLYGONHEADER, TT_POLYGON_TYPE, TT_PRIM_CSPLINE, TT_PRIM_LINE,
    TT_PRIM_QSPLINE,
};

#[cfg(windows)]
use crate::error::RenderError;

#[cfg(windows)]
#[derive(Debug, Clone)]
pub struct GlyphOutline {
    pub path: Option<Path>,
    pub advance: f32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub black_box: (u32, u32),
}

#[cfg(windows)]
#[derive(Debug, Default)]
pub struct GlyphCache {
    glyphs: HashMap<char, GlyphOutline>,
    hits: u32,
    misses: u32,
}

#[cfg(windows)]
impl GlyphCache {
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    pub fn hits(&self) -> u32 {
        self.hits
    }

    pub fn misses(&self) -> u32 {
        self.misses
    }

    pub fn get(&self, ch: char) -> Option<&GlyphOutline> {
        self.glyphs.get(&ch)
    }

    pub fn insert(&mut self, ch: char, glyph: GlyphOutline) {
        self.misses += 1;
        self.glyphs.insert(ch, glyph);
    }

    pub fn mark_hit(&mut self) {
        self.hits += 1;
    }
}

#[cfg(windows)]
fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let b = bytes.get(at..at + 2)?;
    Some(u16::from_ne_bytes([b[0], b[1]]))
}

#[cfg(windows)]
fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 4)?;
    Some(u32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(windows)]
fn read_fixed(bytes: &[u8], at: usize) -> Option<f32> {
    let fract = read_u16(bytes, at)?;
    let value = read_u16(bytes, at + 2)? as i16;
    Some(value as f32 + fract as f32 / 65536.0)
}

/// 读取 POINTFX；GDI 字形坐标为 y 向上，tiny-skia 为 y 向下，这里翻转。
#[cfg(windows)]
fn read_point(bytes: &[u8], at: usize) -> Option<(f32, f32)> {
    let x = read_fixed(bytes, at)?;
    let y = read_fixed(bytes, at + 4)?;
    Some((x, -y))
}

#[cfg(windows)]
pub fn parse_glyph_outline(
    buffer: &[u8],
    metrics: &GLYPHMETRICS,
) -> Result<GlyphOutline, RenderError> {
    let mut builder = PathBuilder::new();
    let mut offset = 0usize;
    let header_bytes = std::mem::size_of::<TTPOLYGONHEADER>();
    let point_bytes = std::mem::size_of::<POINTFX>();
    while offset + header_bytes <= buffer.len() {
        let cb = read_u32(buffer, offset).unwrap_or(0) as usize;
        let dw_type = read_u32(buffer, offset + 4).unwrap_or(0);
        if dw_type != TT_POLYGON_TYPE || cb < header_bytes {
            offset += 1;
            continue;
        }
        let end = offset + cb;
        if end > buffer.len() {
            break;
        }
        let start = read_point(buffer, offset + 8).unwrap_or((0.0, 0.0));
        builder.move_to(start.0, start.1);
        let mut p = offset + header_bytes;
        while p + 4 <= end {
            let w_type = read_u16(buffer, p).unwrap_or(0) as u32;
            let count = read_u16(buffer, p + 2).unwrap_or(0) as usize;
            let size = match count
                .checked_mul(point_bytes)
                .and_then(|n| n.checked_add(4))
            {
                Some(size) => size,
                None => break,
            };
            if count == 0 || p + size > end || p + size > buffer.len() {
                break;
            }
            let mut points = Vec::with_capacity(count);
            for i in 0..count {
                let at = p + 4 + i * point_bytes;
                points.push(read_point(buffer, at).unwrap_or((0.0, 0.0)));
            }
            match w_type {
                TT_PRIM_LINE => {
                    for (x, y) in points {
                        builder.line_to(x, y);
                    }
                }
                TT_PRIM_QSPLINE => {
                    // TrueType quadratic: 连续控制点之间取中点作为曲线终点。
                    for i in 0..points.len() {
                        let (cx, cy) = points[i];
                        let (end_x, end_y) = if i + 1 < points.len() {
                            let (nx, ny) = points[i + 1];
                            ((cx + nx) * 0.5, (cy + ny) * 0.5)
                        } else {
                            (cx, cy)
                        };
                        builder.quad_to(cx, cy, end_x, end_y);
                    }
                }
                TT_PRIM_CSPLINE => {
                    // 少见的 cubic；逐点直线近似，保证不 panic、不越界。
                    for (x, y) in points {
                        builder.line_to(x, y);
                    }
                }
                _ => {}
            }
            p += size;
        }
        builder.close();
        offset = end;
    }
    let path = builder.finish();
    Ok(GlyphOutline {
        path,
        advance: metrics.gmCellIncX.max(0) as f32,
        origin_x: metrics.gmptGlyphOrigin.x,
        origin_y: metrics.gmptGlyphOrigin.y,
        black_box: (metrics.gmBlackBoxX, metrics.gmBlackBoxY),
    })
}

#[cfg(windows)]
pub fn identity_mat2() -> MAT2 {
    MAT2 {
        eM11: windows::Win32::Graphics::Gdi::FIXED { fract: 0, value: 1 },
        eM12: Default::default(),
        eM21: Default::default(),
        eM22: windows::Win32::Graphics::Gdi::FIXED { fract: 0, value: 1 },
    }
}

#[cfg(windows)]
pub fn glyph_error() -> u32 {
    0xFFFF_FFFF
}

#[cfg(all(windows, test))]
mod tests {
    use super::*;

    fn dummy(advance: f32) -> GlyphOutline {
        GlyphOutline {
            path: None,
            advance,
            origin_x: 0,
            origin_y: 0,
            black_box: (0, 0),
        }
    }

    fn metrics(cell_inc: i16) -> GLYPHMETRICS {
        GLYPHMETRICS {
            gmCellIncX: cell_inc,
            ..GLYPHMETRICS::default()
        }
    }

    fn push_u16(bytes: &mut Vec<u8>, v: u16) {
        bytes.extend_from_slice(&v.to_ne_bytes());
    }

    #[test]
    fn cache_tracks_hits_misses_and_entries() {
        let mut cache = GlyphCache::default();
        assert_eq!(cache.len(), 0);
        assert!(cache.get('A').is_none());

        cache.insert('A', dummy(7.0));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.misses(), 1);
        assert_eq!(cache.get('A').map(|g| g.advance), Some(7.0));

        cache.mark_hit();
        cache.mark_hit();
        assert_eq!(cache.hits(), 2);
        assert_eq!(cache.misses(), 1);
    }

    #[test]
    fn empty_buffer_yields_no_path_but_keeps_advance() {
        let m = metrics(9);
        let glyph = parse_glyph_outline(&[], &m).expect("parse");
        assert!(glyph.path.is_none());
        assert_eq!(glyph.advance, 9.0);
    }

    #[test]
    fn garbage_buffer_does_not_panic() {
        let m = metrics(4);
        let mut buffer = vec![0xFFu8; 64];
        buffer[0..4].copy_from_slice(&u32::MAX.to_ne_bytes()); // cb 超出缓冲区
        buffer[4..8].copy_from_slice(&TT_POLYGON_TYPE.to_ne_bytes());
        let glyph = parse_glyph_outline(&buffer, &m).expect("parse");
        assert!(glyph.path.is_none());
        assert_eq!(glyph.advance, 4.0);

        let mut truncated = vec![0u8; 15];
        truncated[0..4].copy_from_slice(&64u32.to_ne_bytes());
        truncated[4..8].copy_from_slice(&TT_POLYGON_TYPE.to_ne_bytes());
        assert!(parse_glyph_outline(&truncated, &m).is_ok());
    }

    #[test]
    fn line_contour_becomes_path() {
        let m = metrics(5);
        let mut bytes = Vec::new();
        // TTPOLYGONHEADER：cb=36、dwType=24、起点 (0,0)（注意 cb 在 dwType 之前）
        push_u16(&mut bytes, 36); // cb 低 16 位
        push_u16(&mut bytes, 0); // cb 高 16 位
        push_u16(&mut bytes, 24); // dwType 低 16 位
        push_u16(&mut bytes, 0); // dwType 高 16 位
        for _ in 0..4 {
            push_u16(&mut bytes, 0); // pfxStart = (0,0)
        }
        // TTPOLYCURVE：wType=TT_PRIM_LINE、cpfx=2、点 (1,1)、(2,2)
        push_u16(&mut bytes, TT_PRIM_LINE as u16);
        push_u16(&mut bytes, 2);
        for value in [0u16, 1, 0, 1, 0, 2, 0, 2] {
            push_u16(&mut bytes, value); // FIXED.fract/value
        }
        let glyph = parse_glyph_outline(&bytes, &m).expect("parse");
        let path = glyph.path.expect("应生成路径");
        assert!(path.segments().count() >= 3);
        assert_eq!(glyph.advance, 5.0);
    }

    #[test]
    fn identity_matrix_is_identity() {
        let mat = identity_mat2();
        assert_eq!(mat.eM11.value, 1);
        assert_eq!(mat.eM22.value, 1);
        assert_eq!(mat.eM12.value, 0);
        assert_eq!(mat.eM21.value, 0);
    }
}
