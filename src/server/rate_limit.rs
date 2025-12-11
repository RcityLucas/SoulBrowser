use std::env;
use std::time::{Duration, Instant};

use dashmap::DashMap;

#[derive(Clone, Copy, Debug)]
pub(crate) enum RateLimitKind {
    Chat,
    Task,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RateLimitConfig {
    pub(crate) chat_per_min: u32,
    pub(crate) task_per_min: u32,
}

impl RateLimitConfig {
    pub(crate) fn from_env(
        chat_env: &str,
        task_env: &str,
        default_chat: u32,
        default_task: u32,
    ) -> Self {
        Self {
            chat_per_min: env_limit(chat_env, default_chat),
            task_per_min: env_limit(task_env, default_task),
        }
    }
}

fn env_limit(name: &str, default: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default)
}

#[derive(Clone)]
pub(crate) struct RateLimiter {
    buckets: DashMap<String, TokenBucket>,
    limits: RateLimitConfig,
}

impl RateLimiter {
    pub(crate) fn new(limits: RateLimitConfig) -> Self {
        Self {
            buckets: DashMap::new(),
            limits,
        }
    }

    pub(crate) fn allow(&self, key: &str, kind: RateLimitKind) -> bool {
        let (capacity, refill) = match kind {
            RateLimitKind::Chat => (
                self.limits.chat_per_min,
                self.limits.chat_per_min as f64 / 60.0,
            ),
            RateLimitKind::Task => (
                self.limits.task_per_min,
                self.limits.task_per_min as f64 / 60.0,
            ),
        };
        if capacity == 0 {
            return true;
        }

        let bucket_key = format!("{}:{kind:?}", key);
        let mut entry = self
            .buckets
            .entry(bucket_key)
            .or_insert_with(|| TokenBucket::new(capacity));
        entry.allow(capacity, refill)
    }

    pub(crate) fn prune_idle(&self, max_idle: Duration) -> usize {
        if max_idle.is_zero() {
            return 0;
        }
        let now = Instant::now();
        let stale: Vec<String> = self
            .buckets
            .iter()
            .filter_map(|entry| {
                if entry.value().is_idle(now, max_idle) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();
        let mut removed = 0;
        for key in stale {
            if self.buckets.remove(&key).is_some() {
                removed += 1;
            }
        }
        removed
    }
}

#[derive(Clone)]
struct TokenBucket {
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(capacity: u32) -> Self {
        Self {
            tokens: capacity as f64,
            last: Instant::now(),
        }
    }

    fn allow(&mut self, capacity: u32, refill_per_sec: f64) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * refill_per_sec).min(capacity as f64);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn is_idle(&self, now: Instant, max_idle: Duration) -> bool {
        now.duration_since(self.last) >= max_idle
    }
}

#[cfg(test)]
mod tests {
    use super::{RateLimitConfig, RateLimiter, TokenBucket};
    use std::time::{Duration, Instant};

    #[test]
    fn prune_idle_removes_stale_buckets() {
        let limiter = RateLimiter::new(RateLimitConfig {
            chat_per_min: 10,
            task_per_min: 5,
        });
        limiter
            .buckets
            .insert("tenant:Chat".into(), TokenBucket::new(5));
        limiter.buckets.insert(
            "tenant:Task".into(),
            TokenBucket {
                tokens: 0.0,
                last: Instant::now() - Duration::from_secs(600),
            },
        );

        let removed = limiter.prune_idle(Duration::from_secs(300));
        assert_eq!(removed, 1);
        assert!(limiter.buckets.contains_key("tenant:Chat"));
        assert!(!limiter.buckets.contains_key("tenant:Task"));
    }
}
