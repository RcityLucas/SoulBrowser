use crate::errors::{SnapErrKind, SnapError};
use crate::model::{ImageBuf, Rect};
use crate::policy::{PixelCfg, StructCfg};

pub fn check_struct(cfg: &StructCfg, estimated_bytes: usize) -> Result<(), SnapError> {
    if estimated_bytes as u64 > cfg.max_bytes_total {
        return Err(SnapErrKind::Oversize.into());
    }
    Ok(())
}

pub fn check_clip(
    cfg: &PixelCfg,
    rect: &Rect,
    img: &ImageBuf,
    encoded_bytes: usize,
) -> Result<(), SnapError> {
    if rect.w == 0 || rect.h == 0 {
        return Err(SnapErrKind::Oversize.into());
    }
    if rect.w * rect.h > cfg.max_clip_area {
        return Err(SnapErrKind::Oversize.into());
    }
    if rect.x + rect.w > img.width || rect.y + rect.h > img.height {
        return Err(SnapErrKind::Oversize.into());
    }
    if cfg.forbid_fullpage && rect.w == img.width && rect.h == img.height {
        return Err(SnapErrKind::Oversize.into());
    }
    if encoded_bytes > cfg.max_bytes_per_clip {
        return Err(SnapErrKind::Oversize.into());
    }
    Ok(())
}
