use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::model::{AnchorResolution, DomAxSnapshot};

#[derive(Default)]
pub struct AnchorCache {
    entries: DashMap<String, (AnchorResolution, Instant)>,
    ttl_ms: AtomicU64,
}

impl AnchorCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: DashMap::new(),
            ttl_ms: AtomicU64::new(duration_to_millis(ttl)),
        }
    }

    pub fn put(&self, key: String, resolution: AnchorResolution) {
        self.entries.insert(key, (resolution, Instant::now()));
    }

    pub fn set_ttl(&self, ttl: Duration) {
        self.ttl_ms
            .store(duration_to_millis(ttl), Ordering::Relaxed);
    }

    pub fn get(&self, key: &str, ttl_override: Option<Duration>) -> Option<AnchorResolution> {
        let ttl = ttl_override.unwrap_or_else(|| self.current_ttl());
        if let Some(entry) = self.entries.get(key) {
            if entry.1.elapsed() <= ttl {
                return Some(entry.0.clone());
            }
        }
        self.entries.remove(key);
        None
    }

    pub fn invalidate_prefix(&self, prefix: &str) {
        let keys: Vec<String> = self
            .entries
            .iter()
            .filter(|entry| entry.key().starts_with(prefix))
            .map(|entry| entry.key().clone())
            .collect();
        for key in keys {
            self.entries.remove(&key);
        }
    }

    pub fn clear(&self) {
        self.entries.clear();
    }

    fn current_ttl(&self) -> Duration {
        millis_to_duration(self.ttl_ms.load(Ordering::Relaxed))
    }
}

#[derive(Default)]
pub struct SnapshotCache {
    entries: DashMap<String, (DomAxSnapshot, Instant)>,
    ttl_ms: AtomicU64,
}

impl SnapshotCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: DashMap::new(),
            ttl_ms: AtomicU64::new(duration_to_millis(ttl)),
        }
    }

    pub fn put(&self, key: String, snapshot: DomAxSnapshot) {
        self.entries.insert(key, (snapshot, Instant::now()));
    }

    pub fn set_ttl(&self, ttl: Duration) {
        self.ttl_ms
            .store(duration_to_millis(ttl), Ordering::Relaxed);
    }

    pub fn get(&self, key: &str) -> Option<DomAxSnapshot> {
        let ttl = self.current_ttl();
        if let Some(entry) = self.entries.get(key) {
            if entry.1.elapsed() <= ttl {
                return Some(entry.0.clone());
            }
        }
        self.entries.remove(key);
        None
    }

    pub fn invalidate_prefix(&self, prefix: &str) {
        let keys: Vec<String> = self
            .entries
            .iter()
            .filter(|entry| entry.key().starts_with(prefix))
            .map(|entry| entry.key().clone())
            .collect();
        for key in keys {
            self.entries.remove(&key);
        }
    }

    pub fn clear(&self) {
        self.entries.clear();
    }

    fn current_ttl(&self) -> Duration {
        millis_to_duration(self.ttl_ms.load(Ordering::Relaxed))
    }
}

fn duration_to_millis(duration: Duration) -> u64 {
    let millis = duration.as_millis();
    if millis > u128::from(u64::MAX) {
        u64::MAX
    } else {
        millis as u64
    }
}

fn millis_to_duration(ms: u64) -> Duration {
    Duration::from_millis(ms)
}
