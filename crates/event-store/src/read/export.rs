use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::config::ColdCfg;
use crate::hot::rings::HotRings;
use crate::model::{EventEnvelope, ReadHandle};
use zstd::stream::Decoder;

pub fn collect_range(
    rings: &HotRings,
    cold_cfg: &ColdCfg,
    ts0: DateTime<Utc>,
    ts1: DateTime<Utc>,
) -> io::Result<Vec<EventEnvelope>> {
    let mut events = Vec::new();
    let mut seen = HashSet::new();

    for event in rings.snapshot().into_iter() {
        if event.ts_wall < ts0 || event.ts_wall > ts1 {
            continue;
        }
        seen.insert(event.event_id.clone());
        events.push(event);
    }

    if cold_cfg.enabled {
        let cold_events = read_cold_events(&cold_cfg.root, ts0, ts1)?;
        for event in cold_events {
            if seen.insert(event.event_id.clone()) {
                events.push(event);
            }
        }
    }

    events.sort_by(|a, b| a.ts_wall.cmp(&b.ts_wall));
    Ok(events)
}

pub fn write_export_file(path: &Path, range: &ReadHandle) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&tmp)?;

    let header = serde_json::to_vec(&json!({
        "from": range.from,
        "to": range.to,
        "count": range.events.len(),
    }))
    .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;
    file.write_all(&header)?;
    file.write_all(b"\n")?;

    for event in &range.events {
        let line = serde_json::to_vec(event)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;
        file.write_all(&line)?;
        file.write_all(b"\n")?;
    }

    file.sync_all()?;
    fs::rename(tmp, path)
}

fn read_cold_events(
    root: &Path,
    ts0: DateTime<Utc>,
    ts1: DateTime<Utc>,
) -> io::Result<Vec<EventEnvelope>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut events = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let meta = match fs::metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.is_dir() {
            for entry in fs::read_dir(&path)? {
                let entry = entry?;
                stack.push(entry.path());
            }
        } else {
            read_log_file(&path, ts0, ts1, &mut events)?;
        }
    }
    Ok(events)
}

fn read_log_file(
    path: &PathBuf,
    ts0: DateTime<Utc>,
    ts1: DateTime<Utc>,
    out: &mut Vec<EventEnvelope>,
) -> io::Result<()> {
    let file = File::open(path)?;
    let mut reader: Box<dyn BufRead> =
        if path.extension().and_then(|ext| ext.to_str()) == Some("zst") {
            let decoder = Decoder::new(file)
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;
            Box::new(BufReader::new(decoder))
        } else {
            Box::new(BufReader::new(file))
        };
    let mut buffer = String::new();
    loop {
        buffer.clear();
        let read = reader.read_line(&mut buffer)?;
        if read == 0 {
            break;
        }
        let line = buffer.trim_end_matches('\n');
        let event: EventEnvelope = match serde_json::from_str(line) {
            Ok(event) => event,
            Err(_) => continue,
        };
        if event.ts_wall < ts0 || event.ts_wall > ts1 {
            continue;
        }
        out.push(event);
    }
    Ok(())
}
