//! 分块范围计算。旧系统使用 8192 字节分块。

use std::ops::Range;

use crate::error::ProtocolError;

pub const LEGACY_CHUNK_SIZE: usize = 8192;

pub fn chunk_ranges(total: usize, chunk: usize) -> Result<Vec<Range<usize>>, ProtocolError> {
    if chunk == 0 {
        return Err(ProtocolError::InvalidChunkSize);
    }
    let mut ranges = Vec::with_capacity(total.div_ceil(chunk));
    let mut start = 0;
    while start < total {
        let end = (start + chunk).min(total);
        ranges.push(start..end);
        start = end;
    }
    Ok(ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_frame_is_37_full_chunks_plus_one() {
        let ranges = chunk_ranges(307_200, LEGACY_CHUNK_SIZE).unwrap();
        assert_eq!(ranges.len(), 38);
        assert_eq!(ranges[0], 0..8192);
        assert_eq!(ranges[36], 294_912..303_104);
        assert_eq!(ranges[37], 303_104..307_200);
        assert_eq!(ranges[37].len(), 4_096);
    }

    #[test]
    fn exact_multiple_has_no_extra_chunk() {
        assert_eq!(chunk_ranges(16_384, 8192).unwrap().len(), 2);
    }

    #[test]
    fn zero_or_empty_cases() {
        assert_eq!(chunk_ranges(0, 8192).unwrap().len(), 0);
        assert_eq!(chunk_ranges(1, 0), Err(ProtocolError::InvalidChunkSize));
    }
}
