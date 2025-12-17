use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use tracing::warn;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub namespace: String,
    pub key: String,
    pub tags: Vec<String>,
    pub note: Option<String>,
    pub metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub use_count: u64,
    #[serde(default)]
    pub success_count: u64,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
}

impl MemoryRecord {
    pub fn new(namespace: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            namespace: namespace.into(),
            key: key.into(),
            tags: Vec::new(),
            note: None,
            metadata: None,
            created_at: Utc::now(),
            use_count: 0,
            success_count: 0,
            last_used_at: None,
        }
    }
}

pub struct MemoryCenter {
    inner: Arc<DashMap<String, Vec<MemoryRecord>>>,
    persistence: Option<PersistenceHandle>,
    metrics: MemoryMetrics,
}

impl Default for MemoryCenter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
struct MemoryMetrics {
    lookups: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    stores: AtomicU64,
    deletes: AtomicU64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStatsSnapshot {
    pub total_queries: u64,
    pub hit_queries: u64,
    pub miss_queries: u64,
    pub hit_rate: f64,
    pub stored_records: u64,
    pub deleted_records: u64,
    pub current_records: u64,
    pub template_uses: u64,
    pub template_successes: u64,
    pub template_success_rate: f64,
}

impl MemoryCenter {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            persistence: None,
            metrics: MemoryMetrics::default(),
        }
    }

    pub fn with_persistence(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let inner = Arc::new(DashMap::new());
        let persistence = Some(PersistenceHandle::new(path.clone(), Arc::clone(&inner)));
        let center = Self {
            inner,
            persistence,
            metrics: MemoryMetrics::default(),
        };

        if path.exists() {
            let bytes = fs::read(&path)?;
            if !bytes.is_empty() {
                let records: Vec<MemoryRecord> = serde_json::from_slice(&bytes)
                    .map_err(|err| io::Error::new(ErrorKind::InvalidData, format!("{err}")))?;
                for record in records {
                    center
                        .inner
                        .entry(record.namespace.clone())
                        .or_insert_with(Vec::new)
                        .push(record);
                }
            }
        }

        Ok(center)
    }

    pub fn store(&self, mut record: MemoryRecord) -> MemoryRecord {
        record.created_at = Utc::now();
        let ns = record.namespace.clone();
        self.inner
            .entry(ns)
            .or_insert_with(Vec::new)
            .push(record.clone());
        self.metrics.stores.fetch_add(1, Ordering::Relaxed);
        self.schedule_flush();
        record
    }

    pub fn list(
        &self,
        namespace: Option<&str>,
        tag: Option<&str>,
        limit: Option<usize>,
    ) -> Vec<MemoryRecord> {
        let mut records: Vec<MemoryRecord> = if let Some(ns) = namespace {
            self.inner
                .get(ns)
                .map(|entry| entry.clone())
                .unwrap_or_default()
        } else {
            self.inner
                .iter()
                .flat_map(|entry| entry.value().clone())
                .collect()
        };

        if let Some(tag_filter) = tag {
            let tag = tag_filter.to_ascii_lowercase();
            records.retain(|record| record.tags.iter().any(|t| t.to_ascii_lowercase() == tag));
        }

        records.sort_by_key(|record| record.created_at);
        records.reverse();
        if let Some(limit) = limit {
            records.truncate(limit);
        }
        records
    }

    pub fn get_by_id(&self, id: &str) -> Option<MemoryRecord> {
        let result = self
            .inner
            .iter()
            .find_map(|entry| entry.value().iter().find(|record| record.id == id).cloned());
        self.metrics.record_lookup(result.is_some());
        result
    }

    pub fn get_by_namespace_and_key(&self, namespace: &str, key: &str) -> Option<MemoryRecord> {
        let result = self
            .inner
            .get(namespace)
            .and_then(|records| records.iter().find(|record| record.key == key).cloned());
        self.metrics.record_lookup(result.is_some());
        result
    }

    pub fn remove_by_id(&self, id: &str) -> Option<MemoryRecord> {
        let mut removed: Option<MemoryRecord> = None;
        let mut empty_keys: Vec<String> = Vec::new();

        for mut entry in self.inner.iter_mut() {
            if let Some(pos) = entry.value().iter().position(|record| record.id == id) {
                removed = Some(entry.value_mut().remove(pos));
                if entry.value().is_empty() {
                    empty_keys.push(entry.key().clone());
                }
                break;
            }
        }

        for key in empty_keys {
            self.inner.remove(&key);
        }

        if removed.is_some() {
            self.metrics.deletes.fetch_add(1, Ordering::Relaxed);
            self.schedule_flush();
        }

        removed
    }

    pub fn update_record<F>(&self, id: &str, update: F) -> Option<MemoryRecord>
    where
        F: FnOnce(&mut MemoryRecord),
    {
        let mut updated: Option<MemoryRecord> = None;
        for mut entry in self.inner.iter_mut() {
            if let Some(record) = entry.value_mut().iter_mut().find(|r| r.id == id) {
                update(record);
                updated = Some(record.clone());
                break;
            }
        }

        if updated.is_some() {
            self.schedule_flush();
        }

        updated
    }

    pub fn persist_now(&self) -> io::Result<()> {
        if let Some(handle) = &self.persistence {
            handle.flush_sync()
        } else {
            Ok(())
        }
    }

    pub fn stats_snapshot(&self) -> MemoryStatsSnapshot {
        let total_queries = self.metrics.lookups.load(Ordering::Relaxed);
        let hit_queries = self.metrics.hits.load(Ordering::Relaxed);
        let miss_queries = self.metrics.misses.load(Ordering::Relaxed);
        let stored_records = self.metrics.stores.load(Ordering::Relaxed);
        let deleted_records = self.metrics.deletes.load(Ordering::Relaxed);
        let current_records = self.total_records() as u64;
        let hit_rate = if total_queries == 0 {
            0.0
        } else {
            hit_queries as f64 / total_queries as f64
        };
        let (template_uses, template_successes) = self.template_totals();
        let template_success_rate = if template_uses == 0 {
            0.0
        } else {
            template_successes as f64 / template_uses as f64
        };
        MemoryStatsSnapshot {
            total_queries,
            hit_queries,
            miss_queries,
            hit_rate,
            stored_records,
            deleted_records,
            current_records,
            template_uses,
            template_successes,
            template_success_rate,
        }
    }

    fn total_records(&self) -> usize {
        self.inner.iter().map(|entry| entry.value().len()).sum()
    }

    fn template_totals(&self) -> (u64, u64) {
        let mut uses = 0;
        let mut successes = 0;
        for entry in self.inner.iter() {
            for record in entry.value().iter() {
                uses += record.use_count;
                successes += record.success_count;
            }
        }
        (uses, successes)
    }

    pub fn record_template_applied(&self, id: &str) {
        let _ = self.update_record(id, |record| {
            record.use_count = record.use_count.saturating_add(1);
            record.last_used_at = Some(Utc::now());
        });
    }

    pub fn record_template_success(&self, id: &str) {
        let _ = self.update_record(id, |record| {
            record.success_count = record.success_count.saturating_add(1);
        });
    }

    fn schedule_flush(&self) {
        if let Some(handle) = &self.persistence {
            handle.schedule();
        }
    }

    fn snapshot_records(inner: &DashMap<String, Vec<MemoryRecord>>) -> Vec<MemoryRecord> {
        inner
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }
}

pub fn normalize_tags<I, S>(tags: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    tags.into_iter()
        .filter_map(|tag| {
            let trimmed = tag.as_ref().trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .collect()
}

pub fn normalize_note<S: AsRef<str>>(note: Option<S>) -> Option<String> {
    note.and_then(|value| {
        let trimmed = value.as_ref().trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

pub fn normalize_metadata(metadata: Option<Value>) -> Option<Value> {
    match metadata {
        Some(Value::Null) => None,
        other => other,
    }
}

struct PersistenceHandle {
    tx: mpsc::Sender<PersistenceSignal>,
    path: PathBuf,
    inner: Arc<DashMap<String, Vec<MemoryRecord>>>,
}

impl PersistenceHandle {
    fn new(path: PathBuf, inner: Arc<DashMap<String, Vec<MemoryRecord>>>) -> Self {
        let (tx, rx) = mpsc::channel();
        let worker_path = path.clone();
        let worker_inner = Arc::clone(&inner);
        thread::spawn(move || persistence_worker(worker_path, worker_inner, rx));
        Self { tx, path, inner }
    }

    fn schedule(&self) {
        let _ = self.tx.send(PersistenceSignal);
    }

    fn flush_sync(&self) -> io::Result<()> {
        write_snapshot(&self.path, &self.inner)
    }
}

#[derive(Clone, Copy)]
struct PersistenceSignal;

fn persistence_worker(
    path: PathBuf,
    inner: Arc<DashMap<String, Vec<MemoryRecord>>>,
    rx: mpsc::Receiver<PersistenceSignal>,
) {
    while rx.recv().is_ok() {
        while rx.try_recv().is_ok() {}
        if let Err(err) = write_snapshot(&path, &inner) {
            warn!(error = %err, "memory-center async persist failed");
        }
    }
}

fn write_snapshot(
    path: &PathBuf,
    inner: &Arc<DashMap<String, Vec<MemoryRecord>>>,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let snapshot = MemoryCenter::snapshot_records(inner);
    let json = serde_json::to_vec_pretty(&snapshot)
        .map_err(|err| io::Error::new(ErrorKind::Other, format!("{err}")))?;
    fs::write(path, json)
}

pub type SharedMemoryCenter = Arc<MemoryCenter>;

impl MemoryMetrics {
    fn record_lookup(&self, hit: bool) {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        if hit {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
    }
}
