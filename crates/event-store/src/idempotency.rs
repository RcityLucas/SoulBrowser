use std::collections::{HashSet, VecDeque};

use parking_lot::Mutex;

/// Simplified in-memory idempotency tracker used by the scaffolding implementation.
#[derive(Debug, Default)]
pub struct IdempotencyTracker {
    inner: Mutex<TrackerInner>,
}

#[derive(Debug)]
struct TrackerInner {
    capacity: usize,
    order: VecDeque<String>,
    keys: HashSet<String>,
}

impl Default for TrackerInner {
    fn default() -> Self {
        Self {
            capacity: 1_024,
            order: VecDeque::new(),
            keys: HashSet::new(),
        }
    }
}

impl IdempotencyTracker {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(TrackerInner {
                capacity,
                order: VecDeque::with_capacity(capacity.min(1_048_576)),
                keys: HashSet::with_capacity(capacity.min(1_048_576)),
            }),
        }
    }

    /// Returns true if the key is newly accepted.
    pub fn accept(&self, key: &str) -> bool {
        let mut guard = self.inner.lock();
        if guard.keys.contains(key) {
            return false;
        }
        guard.keys.insert(key.to_owned());
        guard.order.push_back(key.to_owned());
        if guard.order.len() > guard.capacity {
            if let Some(old) = guard.order.pop_front() {
                guard.keys.remove(&old);
            }
        }
        true
    }

    pub fn resize(&self, capacity: usize) {
        let mut guard = self.inner.lock();
        guard.capacity = capacity;
        while guard.order.len() > guard.capacity {
            if let Some(old) = guard.order.pop_front() {
                guard.keys.remove(&old);
            }
        }
    }
}
