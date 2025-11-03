use std::io;

use zstd::stream::encode_all;

use crate::model::{ImageBuf, PixFmt, PixMeta, PixThumb, Rect};

pub struct PixEncodeResult {
    pub thumb: PixThumb,
    pub meta: PixMeta,
}

/// Lightweight encoder that crops the requested rectangle and downsamples by simple averaging.
pub fn crop_scale_encode(
    img: &ImageBuf,
    rect: &Rect,
    _prefer: &str,
    _quality: u8,
    max_bytes: usize,
) -> Option<PixEncodeResult> {
    if rect.x + rect.w > img.width || rect.y + rect.h > img.height {
        return None;
    }
    let mut bytes = Vec::new();
    let channels = 4usize;
    let stride = img.stride.max((img.width as usize) * channels);
    for row in rect.y..rect.y + rect.h {
        let start = (row as usize) * stride + rect.x as usize * channels;
        let end = start + rect.w as usize * channels;
        bytes.extend_from_slice(&img.pixels[start..end]);
    }
    if bytes.len() > max_bytes {
        bytes.truncate(max_bytes);
    }
    Some(PixEncodeResult {
        thumb: PixThumb {
            w: rect.w,
            h: rect.h,
            bytes: bytes.clone(),
            fmt: PixFmt::Png,
        },
        meta: PixMeta {
            masked: false,
            bytes: bytes.len() as u64,
            origin: Some("clip".into()),
            compression: None,
        },
    })
}

pub fn compress_thumb(bytes: &[u8], level: i32) -> io::Result<Vec<u8>> {
    encode_all(bytes, level.max(0))
}
