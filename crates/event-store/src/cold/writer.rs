use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

use chrono::{DateTime, Utc};
use zstd::stream::Encoder;

use crate::cold::rotate;
use crate::config::ColdCfg;
use crate::metrics::EsMetrics;
use crate::model::EventEnvelope;

#[derive(Clone)]
pub struct ColdWriterHandle {
    tx: Sender<Command>,
    metrics: EsMetrics,
}

enum Command {
    Append(EventEnvelope),
    Flush(mpsc::Sender<io::Result<()>>),
    Shutdown,
}

impl ColdWriterHandle {
    pub fn append(&self, event: EventEnvelope) -> io::Result<()> {
        self.tx
            .send(Command::Append(event))
            .map_err(|err| io::Error::new(io::ErrorKind::BrokenPipe, err.to_string()))?;
        self.metrics.cold_queue_inc();
        Ok(())
    }

    pub fn flush(&self) -> io::Result<()> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Command::Flush(tx))
            .map_err(|err| io::Error::new(io::ErrorKind::BrokenPipe, err.to_string()))?;
        rx.recv()
            .unwrap_or_else(|err| Err(io::Error::new(io::ErrorKind::Other, err.to_string())))
    }
}

impl Drop for ColdWriterHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
    }
}

pub fn spawn(cfg: ColdCfg, metrics: EsMetrics) -> Option<ColdWriterHandle> {
    if !cfg.enabled {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let mut state = ColdWriterState::new(cfg, metrics.clone());
    if thread::Builder::new()
        .name("event-cold-writer".into())
        .spawn(move || {
            while let Ok(cmd) = rx.recv() {
                match cmd {
                    Command::Append(event) => {
                        if let Err(err) = state.append(&event) {
                            eprintln!("[event-store][cold] append failed: {err}");
                            state.metrics.record_cold_error();
                        } else {
                            state.metrics.reset_cold_error_alert();
                        }
                        state.metrics.cold_queue_dec();
                    }
                    Command::Flush(reply) => {
                        let res = state.flush();
                        let _ = reply.send(res);
                    }
                    Command::Shutdown => {
                        let _ = state.flush();
                        break;
                    }
                }
            }
        })
        .is_err()
    {
        return None;
    }
    Some(ColdWriterHandle { tx, metrics })
}

struct ColdWriterState {
    cfg: ColdCfg,
    sink: Option<Box<dyn Write + Send>>,
    raw_file: Option<File>,
    current_path: Option<PathBuf>,
    bytes_written: u64,
    started_at: Option<DateTime<Utc>>,
    sequence: u64,
    metrics: EsMetrics,
}

impl ColdWriterState {
    fn new(cfg: ColdCfg, metrics: EsMetrics) -> Self {
        Self {
            cfg,
            sink: None,
            raw_file: None,
            current_path: None,
            bytes_written: 0,
            started_at: None,
            sequence: 0,
            metrics,
        }
    }

    fn append(&mut self, event: &EventEnvelope) -> io::Result<()> {
        self.ensure_writer(event.ts_wall)?;
        let line = serde_json::to_vec(event)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;
        self.write_line(&line)
    }

    fn write_line(&mut self, line: &[u8]) -> io::Result<()> {
        if self.sink.is_none() {
            self.rotate(Utc::now())?;
        }
        let sink = self.sink.as_mut().expect("writer must be ready");
        sink.write_all(line)?;
        sink.write_all(b"\n")?;
        self.bytes_written += line.len() as u64 + 1;
        Ok(())
    }

    fn ensure_writer(&mut self, ts: DateTime<Utc>) -> io::Result<()> {
        let rotate_by_size =
            self.cfg.rotate_bytes > 0 && self.bytes_written >= self.cfg.rotate_bytes;
        let rotate_by_time = self
            .started_at
            .map(|start| {
                self.cfg.rotate_interval_min > 0
                    && ts - start >= chrono::Duration::minutes(self.cfg.rotate_interval_min as i64)
            })
            .unwrap_or(false);
        if self.sink.is_none() || rotate_by_size || rotate_by_time {
            self.rotate(ts)?;
        }
        Ok(())
    }

    fn rotate(&mut self, ts: DateTime<Utc>) -> io::Result<()> {
        if let Some(sink) = self.sink.as_mut() {
            sink.flush()?;
        }
        if let Some(file) = self.raw_file.as_mut() {
            file.sync_all()?;
        }
        self.sink = None;
        self.raw_file = None;

        self.bytes_written = 0;
        self.started_at = Some(ts);
        self.sequence = self.sequence.wrapping_add(1);
        let path = self.build_path(ts);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let sync_file = file.try_clone()?;
        let writer: Box<dyn Write + Send> = if self.cfg.compress {
            let encoder = Encoder::new(file, 3)
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;
            Box::new(encoder.auto_finish())
        } else {
            Box::new(file)
        };
        self.sink = Some(writer);
        self.raw_file = Some(sync_file);
        self.current_path = Some(path);
        let _ = rotate::cleanup(&self.cfg.root, self.cfg.retain_days, self.cfg.retain_gb);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(sink) = self.sink.as_mut() {
            sink.flush()?;
        }
        if let Some(file) = self.raw_file.as_mut() {
            file.sync_all()?;
        }
        Ok(())
    }

    fn build_path(&self, ts: DateTime<Utc>) -> PathBuf {
        let mut path = self.cfg.root.clone();
        path.push(ts.format("%Y").to_string());
        path.push(ts.format("%m").to_string());
        path.push(ts.format("%d").to_string());
        let mut file = format!(
            "app-{}-{:04}.log",
            ts.format("%Y%m%dT%H%M%S"),
            self.sequence
        );
        if self.cfg.compress {
            file.push_str(".zst");
        }
        path.push(file);
        path
    }
}
