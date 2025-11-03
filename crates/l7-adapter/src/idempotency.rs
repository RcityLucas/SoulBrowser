use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::ports::ToolOutcome;

#[derive(Clone, Default)]
pub struct IdempotencyStore {
    entries: DashMap<(String, String), Entry>,
}

#[derive(Clone)]
struct Entry {
    expires_at: Instant,
    outcome: ToolOutcome,
}

impl IdempotencyStore {
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
        }
    }

    pub fn lookup(&self, tenant_id: &str, key: &str) -> Option<ToolOutcome> {
        let now = Instant::now();
        let map_key = (tenant_id.to_string(), key.to_string());
        match self.entries.get(&map_key) {
            Some(entry) if entry.expires_at > now => Some(entry.outcome.clone()),
            Some(_) => {
                self.entries.remove(&map_key);
                None
            }
            None => None,
        }
    }

    pub fn insert(&self, tenant_id: &str, key: String, ttl: Duration, outcome: &ToolOutcome) {
        if ttl.is_zero() {
            return;
        }
        let expires_at = Instant::now() + ttl;
        let entry = Entry {
            expires_at,
            outcome: outcome.clone(),
        };
        self.entries.insert((tenant_id.to_string(), key), entry);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn lookup_returns_cached_outcome() {
        let store = IdempotencyStore::new();
        let outcome = ToolOutcome {
            status: "ok".into(),
            ..ToolOutcome::default()
        };
        store.insert("tenant", "key".into(), Duration::from_secs(5), &outcome);
        let cached = store.lookup("tenant", "key");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().status, "ok");
    }

    #[test]
    fn lookup_expired_entry_is_removed() {
        let store = IdempotencyStore::new();
        let outcome = ToolOutcome {
            status: "ok".into(),
            ..ToolOutcome::default()
        };
        store.insert("tenant", "key".into(), Duration::from_millis(1), &outcome);
        std::thread::sleep(Duration::from_millis(5));
        assert!(store.lookup("tenant", "key").is_none());
        assert_eq!(store.len(), 0);
    }
}
