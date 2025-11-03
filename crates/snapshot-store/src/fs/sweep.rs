use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

pub fn sweep_dir(root: &Path, ttl: Duration) -> io::Result<usize> {
    if !root.exists() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let modified = metadata.modified().unwrap_or(SystemTime::now());
        if SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default()
            > ttl
        {
            if metadata.is_file() {
                fs::remove_file(entry.path())?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}
