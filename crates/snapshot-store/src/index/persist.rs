use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::model::SnapRef;

pub fn load(path: &PathBuf) -> io::Result<Option<SnapRef>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read(path)?;
    let snap = serde_json::from_slice(&raw).ok();
    Ok(snap)
}

pub fn store(path: &PathBuf, snap: &SnapRef) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("tmp");
    let mut file = fs::File::create(&temp)?;
    let data = serde_json::to_vec_pretty(snap).unwrap_or_default();
    file.write_all(&data)?;
    file.sync_all()?;
    fs::rename(temp, path)
}
