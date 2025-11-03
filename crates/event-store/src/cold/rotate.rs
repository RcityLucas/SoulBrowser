use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

/// Rotation helper that enforces TTL and total storage limits for cold logs.
pub fn cleanup(root: &Path, retain_days: u32, retain_gb: u64) -> io::Result<()> {
    if !root.exists() {
        return Ok(());
    }

    let mut entries = Vec::new();
    gather_entries(root, &mut entries)?;

    if retain_days > 0 {
        let now = Utc::now();
        for entry in &entries {
            if (now - entry.modified).num_days() > retain_days as i64 {
                let _ = fs::remove_file(&entry.path);
            }
        }
    }

    let retain_bytes = retain_gb.saturating_mul(1024 * 1024 * 1024);
    if retain_bytes > 0 {
        entries.clear();
        gather_entries(root, &mut entries)?;
        entries.sort_by(|a, b| a.modified.cmp(&b.modified));
        let mut total: u128 = entries.iter().map(|e| e.size as u128).sum();
        for entry in &entries {
            if total <= retain_bytes as u128 {
                break;
            }
            if fs::remove_file(&entry.path).is_ok() {
                total = total.saturating_sub(entry.size as u128);
            }
        }
    }

    prune_dirs(root, true)?;
    Ok(())
}

struct FileEntry {
    path: PathBuf,
    size: u64,
    modified: DateTime<Utc>,
}

fn gather_entries(path: &Path, out: &mut Vec<FileEntry>) -> io::Result<()> {
    if !path.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let entry_path = entry.path();
        if metadata.is_dir() {
            gather_entries(&entry_path, out)?;
            continue;
        }
        let size = metadata.len();
        let modified = metadata
            .modified()
            .unwrap_or_else(|_| std::time::SystemTime::now());
        let modified = DateTime::<Utc>::from(modified);
        out.push(FileEntry {
            path: entry_path,
            size,
            modified,
        });
    }
    Ok(())
}

fn prune_dirs(path: &Path, is_root: bool) -> io::Result<bool> {
    if !path.is_dir() {
        return Ok(false);
    }
    let mut is_empty = true;
    let entries: Vec<_> = fs::read_dir(path)?.collect();
    for entry in entries {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            if !prune_dirs(&entry_path, false)? {
                is_empty = false;
            }
        } else {
            is_empty = false;
        }
    }
    if !is_root && is_empty {
        let _ = fs::remove_dir(path);
    }
    Ok(is_empty)
}
