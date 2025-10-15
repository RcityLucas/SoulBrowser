use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::model::{AnchorResolution, DomAxSnapshot};

#[derive(Default)]
pub struct AnchorCache {
    entries: DashMap<String, (AnchorResolution, Instant)>,
    ttl: Duration,
}

impl AnchorCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: DashMap::new(),
            ttl,
        }
    }

    pub fn put(&self, key: String, resolution: AnchorResolution) {
        self.entries.insert(key, (resolution, Instant::now()));
    }

    pub fn get(&self, key: &str) -> Option<AnchorResolution> {
        if let Some(entry) = self.entries.get(key) {
            if entry.1.elapsed() <= self.ttl {
                return Some(entry.0.clone());
            }
        }
        self.entries.remove(key);
        None
    }
}

#[derive(Default)]
pub struct SnapshotCache {
    entries: DashMap<String, (DomAxSnapshot, Instant)>,
    ttl: Duration,
}

impl SnapshotCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: DashMap::new(),
            ttl,
        }
    }

    pub fn put(&self, key: String, snapshot: DomAxSnapshot) {
        self.entries.insert(key, (snapshot, Instant::now()));
    }

    pub fn get(&self, key: &str) -> Option<DomAxSnapshot> {
        if let Some(entry) = self.entries.get(key) {
            if entry.1.elapsed() <= self.ttl {
                return Some(entry.0.clone());
            }
        }
        self.entries.remove(key);
        None
    }
}
