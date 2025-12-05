use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
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

#[derive(Default)]
pub struct MemoryCenter {
    inner: DashMap<String, Vec<MemoryRecord>>,
    storage_path: Option<PathBuf>,
    metrics: MemoryMetrics,
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
            inner: DashMap::new(),
            storage_path: None,
            metrics: MemoryMetrics::default(),
        }
    }

    pub fn with_persistence(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let center = Self {
            inner: DashMap::new(),
            storage_path: Some(path.clone()),
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
        if let Err(err) = self.persist_to_disk() {
            warn!(error = %err, "memory-center persist failed after store");
        }
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
            if let Err(err) = self.persist_to_disk() {
                warn!(error = %err, "memory-center persist failed after delete");
            }
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
            if let Err(err) = self.persist_to_disk() {
                warn!(error = %err, "memory-center persist failed after update");
            }
        }

        updated
    }

    pub fn persist_now(&self) -> io::Result<()> {
        self.persist_to_disk()
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

    fn persist_to_disk(&self) -> io::Result<()> {
        let Some(path) = self.storage_path.as_ref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut all_records: Vec<MemoryRecord> = Vec::new();
        for entry in self.inner.iter() {
            all_records.extend(entry.value().clone());
        }
        let json = serde_json::to_vec_pretty(&all_records)
            .map_err(|err| io::Error::new(ErrorKind::Other, format!("{err}")))?;
        fs::write(path, json)
    }
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
