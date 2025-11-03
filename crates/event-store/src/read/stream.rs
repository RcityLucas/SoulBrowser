use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::config::ColdCfg;
use crate::hot::rings::HotRings;
use crate::model::{EventEnvelope, StreamBatch};
use zstd::stream::Decoder;

const DEFAULT_PAGE_SIZE: usize = 256;

/// Cursor that streams a time-bounded range of events in bounded pages.
pub struct EventStreamCursor {
    page_size: usize,
    state: RangeStream,
}

impl EventStreamCursor {
    pub fn new(
        rings: Arc<HotRings>,
        cold_cfg: ColdCfg,
        ts0: DateTime<Utc>,
        ts1: DateTime<Utc>,
        page_size: usize,
    ) -> io::Result<Self> {
        let page_size = if page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            page_size.min(16_384)
        };
        let state = RangeStream::new(rings, cold_cfg, ts0, ts1)?;
        Ok(Self { page_size, state })
    }

    /// Fetches the next page. Returns `Ok(None)` when the range is fully consumed.
    pub fn next_page(&mut self) -> io::Result<Option<StreamBatch>> {
        let Some(events) = self.state.next_batch(self.page_size)? else {
            return Ok(None);
        };
        let is_last = self.state.is_finished();
        Ok(Some(StreamBatch { events, is_last }))
    }
}

struct RangeStream {
    hot: HotIter,
    cold: Option<ColdIter>,
    peek_hot: Option<EventEnvelope>,
    peek_cold: Option<EventEnvelope>,
    seen: HashSet<String>,
}

impl RangeStream {
    fn new(
        rings: Arc<HotRings>,
        cold_cfg: ColdCfg,
        ts0: DateTime<Utc>,
        ts1: DateTime<Utc>,
    ) -> io::Result<Self> {
        let hot = HotIter::new(rings, ts0, ts1);
        let cold = ColdIter::new(cold_cfg, ts0, ts1)?;
        Ok(Self {
            hot,
            cold,
            peek_hot: None,
            peek_cold: None,
            seen: HashSet::new(),
        })
    }

    fn next_batch(&mut self, max_items: usize) -> io::Result<Option<Vec<EventEnvelope>>> {
        if max_items == 0 {
            return Ok(None);
        }
        let mut batch = Vec::with_capacity(max_items);
        while batch.len() < max_items {
            let Some(event) = self.next_event()? else {
                break;
            };
            batch.push(event);
        }
        if batch.is_empty() {
            Ok(None)
        } else {
            Ok(Some(batch))
        }
    }

    fn next_event(&mut self) -> io::Result<Option<EventEnvelope>> {
        loop {
            if self.peek_hot.is_none() {
                self.peek_hot = self.hot.next();
            }
            if self.peek_cold.is_none() {
                if let Some(cold) = self.cold.as_mut() {
                    self.peek_cold = cold.next_event()?;
                }
            }

            match (&self.peek_hot, &self.peek_cold) {
                (None, None) => return Ok(None),
                (Some(_), None) => {
                    let event = self.peek_hot.take().unwrap();
                    if self.seen.insert(event.event_id.clone()) {
                        return Ok(Some(event));
                    }
                }
                (None, Some(_)) => {
                    let event = self.peek_cold.take().unwrap();
                    if self.seen.insert(event.event_id.clone()) {
                        return Ok(Some(event));
                    }
                }
                (Some(hot), Some(cold)) => {
                    if hot.ts_wall <= cold.ts_wall {
                        let event = self.peek_hot.take().unwrap();
                        if self.seen.insert(event.event_id.clone()) {
                            return Ok(Some(event));
                        }
                    } else {
                        let event = self.peek_cold.take().unwrap();
                        if self.seen.insert(event.event_id.clone()) {
                            return Ok(Some(event));
                        }
                    }
                }
            }
        }
    }

    fn is_finished(&self) -> bool {
        self.peek_hot.is_none()
            && self.peek_cold.is_none()
            && self.hot.is_finished()
            && self.cold.as_ref().map(|c| c.is_finished()).unwrap_or(true)
    }
}

struct HotIter {
    events: Vec<EventEnvelope>,
    index: usize,
}

impl HotIter {
    fn new(rings: Arc<HotRings>, ts0: DateTime<Utc>, ts1: DateTime<Utc>) -> Self {
        let mut events: Vec<EventEnvelope> = rings
            .snapshot()
            .into_iter()
            .filter(|event| event.ts_wall >= ts0 && event.ts_wall <= ts1)
            .collect();
        events.sort_by(|a, b| a.ts_wall.cmp(&b.ts_wall));
        Self { events, index: 0 }
    }

    fn next(&mut self) -> Option<EventEnvelope> {
        if self.index >= self.events.len() {
            return None;
        }
        let event = self.events[self.index].clone();
        self.index += 1;
        Some(event)
    }

    fn is_finished(&self) -> bool {
        self.index >= self.events.len()
    }
}

struct ColdIter {
    files: Vec<PathBuf>,
    file_idx: usize,
    current: Option<FileLines>,
    ts0: DateTime<Utc>,
    ts1: DateTime<Utc>,
}

impl ColdIter {
    fn new(cfg: ColdCfg, ts0: DateTime<Utc>, ts1: DateTime<Utc>) -> io::Result<Option<Self>> {
        if !cfg.enabled {
            return Ok(None);
        }
        if !cfg.root.exists() {
            return Ok(None);
        }
        let mut files = collect_files(&cfg.root)?;
        files.sort();
        Ok(Some(Self {
            files,
            file_idx: 0,
            current: None,
            ts0,
            ts1,
        }))
    }

    fn next_event(&mut self) -> io::Result<Option<EventEnvelope>> {
        loop {
            if self.current.is_none() {
                if self.file_idx >= self.files.len() {
                    return Ok(None);
                }
                let path = self.files[self.file_idx].clone();
                self.file_idx += 1;
                match open_reader(&path) {
                    Ok(reader) => {
                        self.current = Some(FileLines::new(reader));
                    }
                    Err(_) => continue,
                }
            }

            if let Some(file) = self.current.as_mut() {
                match file.next_line()? {
                    Some(line) => {
                        let event: EventEnvelope = match serde_json::from_str(&line) {
                            Ok(event) => event,
                            Err(_) => continue,
                        };
                        if event.ts_wall < self.ts0 || event.ts_wall > self.ts1 {
                            continue;
                        }
                        return Ok(Some(event));
                    }
                    None => {
                        self.current = None;
                    }
                }
            }
        }
    }

    fn is_finished(&self) -> bool {
        self.current.is_none() && self.file_idx >= self.files.len()
    }
}

struct FileLines {
    reader: Box<dyn BufRead + Send>,
    buffer: String,
}

impl FileLines {
    fn new(reader: Box<dyn BufRead + Send>) -> Self {
        Self {
            reader,
            buffer: String::new(),
        }
    }

    fn next_line(&mut self) -> io::Result<Option<String>> {
        self.buffer.clear();
        let read = self.reader.read_line(&mut self.buffer)?;
        if read == 0 {
            return Ok(None);
        }
        Ok(Some(self.buffer.trim_end_matches('\n').to_string()))
    }
}

fn open_reader(path: &Path) -> io::Result<Box<dyn BufRead + Send>> {
    let file = File::open(path)?;
    if path.extension().and_then(|ext| ext.to_str()) == Some("zst") {
        let decoder = Decoder::new(file)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;
        Ok(Box::new(BufReader::new(decoder)))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

fn collect_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.is_dir() {
            for entry in std::fs::read_dir(&path)? {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => continue,
                };
                stack.push(entry.path());
            }
        } else {
            files.push(path);
        }
    }
    Ok(files)
}
