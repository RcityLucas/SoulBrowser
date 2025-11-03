use std::panic;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use parking_lot::RwLock;
use tokio::task;
use uuid::Uuid;

use crate::cold::writer::{self, ColdWriterHandle};
use crate::config::EsPolicyView;
use crate::errors::{EsError, EsErrorKind};
use crate::hot::rings::HotRings;
use crate::hot::writer::HotWriter;
use crate::metrics::EsMetrics;
use crate::model::{
    AppendAck, AppendMeta, BatchAck, EventEnvelope, Filter, Observation, ReadHandle, ReplayBundle,
};
use crate::read::{export, replay, stream};
use crate::{drop_policy, idempotency::IdempotencyTracker, redact};

pub type EventStoreResult<T> = Result<T, EsError>;
pub type PostHook = Arc<dyn Fn(&EventEnvelope) + Send + Sync + 'static>;

#[async_trait]
pub trait EventStore: Send + Sync {
    fn register_post_hook(&self, hook: PostHook);
    fn reload_policy(&self, policy: EsPolicyView) -> EventStoreResult<()>;
    async fn append_observation(
        &self,
        obs: Observation,
        meta: AppendMeta,
    ) -> EventStoreResult<AppendAck>;
    async fn append_event(
        &self,
        env: EventEnvelope,
        meta: AppendMeta,
    ) -> EventStoreResult<AppendAck>;
    async fn batch_append(
        &self,
        envs: Vec<(EventEnvelope, AppendMeta)>,
    ) -> EventStoreResult<BatchAck>;
    async fn flush(&self) -> EventStoreResult<()>;

    async fn tail(
        &self,
        limit: usize,
        filter: Option<Filter>,
    ) -> EventStoreResult<Vec<EventEnvelope>>;
    async fn since(
        &self,
        ts_wall: chrono::DateTime<chrono::Utc>,
        limit: usize,
        filter: Option<Filter>,
    ) -> EventStoreResult<Vec<EventEnvelope>>;
    async fn by_action(&self, action_id: &str) -> EventStoreResult<Vec<EventEnvelope>>;
    async fn export_range(
        &self,
        ts0: chrono::DateTime<chrono::Utc>,
        ts1: chrono::DateTime<chrono::Utc>,
    ) -> EventStoreResult<ReadHandle>;
    async fn export_range_to_file(
        &self,
        ts0: chrono::DateTime<chrono::Utc>,
        ts1: chrono::DateTime<chrono::Utc>,
        path: &Path,
    ) -> EventStoreResult<()>;
    async fn replay_minimal(&self, action_id: &str) -> EventStoreResult<ReplayBundle>;
    async fn stream_range(
        &self,
        ts0: chrono::DateTime<chrono::Utc>,
        ts1: chrono::DateTime<chrono::Utc>,
        page_size: usize,
    ) -> EventStoreResult<stream::EventStreamCursor>;
}

pub struct InMemoryEventStore {
    policy: RwLock<EsPolicyView>,
    hot_writer: RwLock<HotWriter>,
    hot_rings: RwLock<Arc<HotRings>>,
    cold: RwLock<Option<ColdWriterHandle>>,
    idempotency: IdempotencyTracker,
    metrics: EsMetrics,
    hooks: HookRegistry,
}

impl InMemoryEventStore {
    pub fn new(policy: EsPolicyView) -> Arc<Self> {
        let hot_cfg = policy.hot.clone();
        let cold_cfg = policy.cold.clone();
        let hot_writer = HotWriter::new(hot_cfg);
        let hot_rings = hot_writer.rings();
        let metrics = EsMetrics::default();
        let cold = writer::spawn(cold_cfg, metrics.clone());
        Arc::new(Self {
            idempotency: IdempotencyTracker::with_capacity(policy.idempotency.lru_capacity),
            metrics,
            cold: RwLock::new(cold),
            hot_rings: RwLock::new(hot_rings),
            hot_writer: RwLock::new(hot_writer),
            hooks: HookRegistry::default(),
            policy: RwLock::new(policy),
        })
    }

    fn apply_redaction(&self, mut env: EventEnvelope, policy: &EsPolicyView) -> EventEnvelope {
        redact::apply(&mut env, &policy.redact, policy.hot.max_payload_bytes);
        env
    }

    fn utilization(&self) -> f32 {
        self.hot_rings.read().utilization()
    }

    pub fn metrics(&self) -> EsMetrics {
        self.metrics.clone()
    }

    pub fn register_post_hook_fn<F>(&self, hook: F)
    where
        F: Fn(&EventEnvelope) + Send + Sync + 'static,
    {
        self.hooks.register(Arc::new(hook));
    }

    pub fn apply_policy(&self, policy: EsPolicyView) -> EventStoreResult<()> {
        let mut current = self.policy.write();
        let existing = current.clone();

        if existing.hot != policy.hot {
            let new_writer = HotWriter::new(policy.hot.clone());
            let new_rings = new_writer.rings();
            *self.hot_writer.write() = new_writer;
            *self.hot_rings.write() = new_rings;
        }

        if existing.cold != policy.cold {
            let mut cold_guard = self.cold.write();
            if policy.cold.enabled {
                let new_handle = writer::spawn(policy.cold.clone(), self.metrics.clone());
                if let Some(handle) = new_handle {
                    *cold_guard = Some(handle);
                } else {
                    return Err(EsErrorKind::Internal("failed to spawn cold writer".into()).into());
                }
            } else {
                *cold_guard = None;
            }
        }

        if existing.idempotency.lru_capacity != policy.idempotency.lru_capacity {
            self.idempotency.resize(policy.idempotency.lru_capacity);
        }

        *current = policy;
        Ok(())
    }
}

#[async_trait]
impl EventStore for InMemoryEventStore {
    async fn append_observation(
        &self,
        obs: Observation,
        meta: AppendMeta,
    ) -> EventStoreResult<AppendAck> {
        let env = EventEnvelope {
            event_id: meta
                .idempotency_key
                .clone()
                .unwrap_or_else(|| format!("obs-{}", Uuid::new_v4())),
            ts_mono: obs
                .meta
                .get("ts_mono")
                .and_then(|v| v.as_u64())
                .unwrap_or_default() as u128,
            ts_wall: chrono::Utc::now(),
            scope: meta.scope_hint(),
            source: crate::model::EventSource::L3,
            kind: "OBSERVATION".into(),
            level: crate::model::LogLevel::Info,
            payload: serde_json::to_value(obs).unwrap_or_default(),
            artifacts: Vec::new(),
            tags: Vec::new(),
        };
        self.append_event(env, meta).await
    }

    async fn append_event(
        &self,
        env: EventEnvelope,
        meta: AppendMeta,
    ) -> EventStoreResult<AppendAck> {
        let key = meta
            .idempotency_key
            .clone()
            .unwrap_or_else(|| env.idempotency_hint());
        if !self.idempotency.accept(&key) {
            self.metrics.record_append_drop("duplicate");
            return Ok(AppendAck {
                event_id: env.event_id,
                accepted: false,
                dropped_reason: Some("duplicate".into()),
            });
        }

        let policy_snapshot = self.policy.read();
        let env = self.apply_redaction(env, &policy_snapshot);
        if drop_policy::should_drop(&env, self.utilization(), &policy_snapshot.drop) {
            self.metrics.record_append_drop("policy");
            return Ok(AppendAck {
                event_id: env.event_id,
                accepted: false,
                dropped_reason: Some("dropped_by_policy".into()),
            });
        }
        drop(policy_snapshot);

        let event_id = env.event_id.clone();
        {
            let writer = self.hot_writer.read();
            writer.write(env.clone());
        }
        self.metrics.record_append_ok();
        self.metrics.observe_hot_utilization(self.utilization());

        if !meta.hot_only {
            if let Some(cold) = self.cold.read().clone() {
                if let Err(err) = cold.append(env.clone()) {
                    self.metrics.record_append_drop("cold_error");
                    self.metrics.record_cold_error();
                    eprintln!("[event-store] cold append failed: {err}");
                }
            }
        }

        self.hooks.emit(&env);

        Ok(AppendAck {
            event_id,
            accepted: true,
            dropped_reason: None,
        })
    }

    async fn batch_append(
        &self,
        envs: Vec<(EventEnvelope, AppendMeta)>,
    ) -> EventStoreResult<BatchAck> {
        let mut accepted = 0;
        let mut dropped = 0;
        for (env, meta) in envs {
            match self.append_event(env, meta).await {
                Ok(ack) if ack.accepted => accepted += 1,
                Ok(_) => dropped += 1,
                Err(err) => return Err(err),
            }
        }
        Ok(BatchAck {
            accepted,
            dropped,
            errors: Default::default(),
        })
    }

    async fn flush(&self) -> EventStoreResult<()> {
        let cold_handle = {
            let guard = self.cold.read();
            guard.clone()
        };
        if let Some(cold) = cold_handle {
            let handle = cold.clone();
            let (result, elapsed) = task::spawn_blocking(move || {
                let start = Instant::now();
                let res = handle.flush();
                (res, start.elapsed())
            })
            .await
            .map_err(|err| EsError::from(EsErrorKind::Internal(err.to_string())))?;
            match result {
                Ok(_) => {
                    self.metrics.record_cold_flush(elapsed.as_millis() as u64);
                    self.metrics.reset_cold_error_alert();
                }
                Err(err) => {
                    self.metrics.record_cold_error();
                    return Err(EsError::from(EsErrorKind::ColdWriteFailed(err.to_string())));
                }
            }
        }
        Ok(())
    }

    async fn tail(
        &self,
        limit: usize,
        filter: Option<Filter>,
    ) -> EventStoreResult<Vec<EventEnvelope>> {
        let rings = self.hot_rings.read().clone();
        Ok(rings.tail(limit, filter.as_ref()))
    }

    async fn since(
        &self,
        ts_wall: chrono::DateTime<chrono::Utc>,
        limit: usize,
        filter: Option<Filter>,
    ) -> EventStoreResult<Vec<EventEnvelope>> {
        let rings = self.hot_rings.read().clone();
        Ok(rings.since(ts_wall, limit, filter.as_ref()))
    }

    async fn by_action(&self, action_id: &str) -> EventStoreResult<Vec<EventEnvelope>> {
        let rings = self.hot_rings.read().clone();
        Ok(rings.by_action(action_id))
    }

    async fn export_range(
        &self,
        ts0: chrono::DateTime<chrono::Utc>,
        ts1: chrono::DateTime<chrono::Utc>,
    ) -> EventStoreResult<ReadHandle> {
        if ts1 < ts0 {
            return Err(EsErrorKind::InvalidFilter("ts1 < ts0".into()).into());
        }
        let rings = self.hot_rings.read().clone();
        let policy = self.policy.read();
        let events = export::collect_range(rings.as_ref(), &policy.cold, ts0, ts1)
            .map_err(|err| EsErrorKind::ColdWriteFailed(err.to_string()))?;
        Ok(ReadHandle {
            from: ts0,
            to: ts1,
            events,
        })
    }

    async fn export_range_to_file(
        &self,
        ts0: chrono::DateTime<chrono::Utc>,
        ts1: chrono::DateTime<chrono::Utc>,
        path: &Path,
    ) -> EventStoreResult<()> {
        let handle = self.export_range(ts0, ts1).await?;
        export::write_export_file(path, &handle)
            .map_err(|err| EsErrorKind::ColdWriteFailed(err.to_string()))?;
        Ok(())
    }

    async fn replay_minimal(&self, action_id: &str) -> EventStoreResult<ReplayBundle> {
        let rings = self.hot_rings.read().clone();
        let events = rings.by_action(action_id);
        Ok(replay::build_minimal(&events))
    }

    async fn stream_range(
        &self,
        ts0: chrono::DateTime<chrono::Utc>,
        ts1: chrono::DateTime<chrono::Utc>,
        page_size: usize,
    ) -> EventStoreResult<stream::EventStreamCursor> {
        if ts1 < ts0 {
            return Err(EsErrorKind::InvalidFilter("ts1 < ts0".into()).into());
        }
        let rings = self.hot_rings.read().clone();
        let policy = self.policy.read();
        let cursor =
            stream::EventStreamCursor::new(rings, policy.cold.clone(), ts0, ts1, page_size)
                .map_err(|err| EsErrorKind::ColdReadFailed(err.to_string()))?;
        Ok(cursor)
    }

    fn register_post_hook(&self, hook: PostHook) {
        self.hooks.register(hook);
    }

    fn reload_policy(&self, policy: EsPolicyView) -> EventStoreResult<()> {
        self.apply_policy(policy)
    }
}

#[derive(Default)]
struct HookRegistry {
    hooks: RwLock<Vec<PostHook>>,
}

impl HookRegistry {
    fn register(&self, hook: PostHook) {
        self.hooks.write().push(hook);
    }

    fn emit(&self, event: &EventEnvelope) {
        let snapshot: Vec<PostHook> = self.hooks.read().iter().cloned().collect();
        for hook in snapshot {
            if panic::catch_unwind(panic::AssertUnwindSafe(|| (hook)(event))).is_err() {
                eprintln!("[event-store] post-hook panicked; continuing");
            }
        }
    }
}

/// Builder helper to simplify initialization while keeping the API extendable.
#[derive(Default)]
pub struct EventStoreBuilder {
    policy: EsPolicyView,
}

impl EventStoreBuilder {
    pub fn new(policy: EsPolicyView) -> Self {
        Self { policy }
    }

    pub fn build(self) -> Arc<dyn EventStore> {
        InMemoryEventStore::new(self.policy)
    }
}

trait MetaExt {
    fn scope_hint(&self) -> crate::model::EventScope;
}

impl MetaExt for AppendMeta {
    fn scope_hint(&self) -> crate::model::EventScope {
        crate::model::EventScope::default()
    }
}
