use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct RuntimeOverrideStore {
    entries: HashMap<String, RuntimeOverrideEntry>,
}

#[derive(Clone, Debug)]
pub struct RuntimeOverrideEntry {
    pub value: Value,
    pub expires_at: Option<Instant>,
}

impl RuntimeOverrideStore {
    pub fn insert(&mut self, key: String, value: Value, ttl: Option<Duration>) {
        let expires_at = ttl.map(|dur| Instant::now() + dur);
        self.entries
            .insert(key, RuntimeOverrideEntry { value, expires_at });
    }

    pub fn remove(&mut self, key: &str) -> bool {
        self.entries.remove(key).is_some()
    }

    pub fn active_entries(&mut self) -> Vec<(String, Value)> {
        let now = Instant::now();
        let mut result = Vec::new();
        self.entries.retain(|key, entry| {
            let active = entry
                .expires_at
                .map(|expires| expires > now)
                .unwrap_or(true);
            if active {
                result.push((key.clone(), entry.value.clone()));
            }
            active
        });
        result
    }
}
