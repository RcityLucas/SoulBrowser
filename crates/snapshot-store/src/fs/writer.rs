use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use soulbrowser_core_types::ActionId;

use crate::model::{PixClip, SnapCtx, StructSnap};
use crate::policy::IoCfg;

pub fn write_struct(cfg: &IoCfg, ctx: &SnapCtx, snap: &StructSnap) -> io::Result<PathBuf> {
    let path = super::layout::struct_path(cfg, ctx, snap.id.trim_start_matches("ss_"));
    let data = serde_json::to_vec(snap)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;
    write_atomic(path, &data)
}

pub fn write_pix(cfg: &IoCfg, ctx: &SnapCtx, clip: &PixClip) -> io::Result<PathBuf> {
    let path = super::layout::pix_path(cfg, ctx, clip.id.trim_start_matches("px_"));
    let data = serde_json::to_vec(clip)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;
    write_atomic(path, &data)
}

pub fn write_action_index(cfg: &IoCfg, action: &ActionId, data: &[u8]) -> io::Result<PathBuf> {
    let path = super::layout::action_index_path(cfg, action);
    write_atomic(path, data)
}

pub fn remove_file(path: &Path) {
    let _ = fs::remove_file(path);
}

fn write_atomic(path: PathBuf, data: &[u8]) -> io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&tmp)?;
    file.write_all(data)?;
    file.sync_all()?;
    fs::rename(tmp, &path)?;
    Ok(path)
}
