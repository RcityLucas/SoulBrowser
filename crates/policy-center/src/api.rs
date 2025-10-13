use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{watch, Mutex};
use tokio::time::sleep;
use tracing::warn;

use crate::errors::PolicyError;
use crate::model::{PolicySnapshot, PolicySource, RuntimeOverrideSpec};
use crate::override_store::RuntimeOverrideStore;

#[async_trait]
pub trait PolicyCenter: Send + Sync {
    async fn snapshot(&self) -> Arc<PolicySnapshot>;
    async fn reload(&self) -> Result<(), PolicyError>;
    async fn apply_override(&self, override_spec: RuntimeOverrideSpec) -> Result<(), PolicyError>;
    fn subscribe(&self) -> watch::Receiver<Arc<PolicySnapshot>>;
    async fn guard(&self) -> PolicyGuard;
}

struct PolicyState {
    base: PolicySnapshot,
    snapshot: PolicySnapshot,
    overrides: RuntimeOverrideStore,
    rev_counter: u64,
}

impl PolicyState {
    fn new(base: PolicySnapshot) -> Self {
        let rev_counter = base.rev;
        Self {
            base: base.clone(),
            snapshot: base,
            overrides: RuntimeOverrideStore::default(),
            rev_counter,
        }
    }

    fn apply_active_overrides(&mut self) -> Result<(), PolicyError> {
        let mut new_snapshot = self.base.clone();
        let entries = self.overrides.active_entries();
        for (path, value) in entries {
            apply_override_to_snapshot(
                &mut new_snapshot,
                &path,
                &value,
                PolicySource::RuntimeOverride,
            )?;
        }
        self.rev_counter = self.rev_counter.saturating_add(1);
        new_snapshot.rev = self.rev_counter;
        self.snapshot = new_snapshot;
        Ok(())
    }
}

pub struct InMemoryPolicyCenter {
    state: Arc<Mutex<PolicyState>>,
    watch_tx: watch::Sender<Arc<PolicySnapshot>>,
}

impl InMemoryPolicyCenter {
    pub fn new(snapshot: PolicySnapshot) -> Self {
        let state = PolicyState::new(snapshot);
        let current_snapshot = Arc::new(state.snapshot.clone());
        let (watch_tx, _watch_rx) = watch::channel(current_snapshot);
        Self {
            state: Arc::new(Mutex::new(state)),
            watch_tx,
        }
    }
}

#[async_trait]
impl PolicyCenter for InMemoryPolicyCenter {
    async fn snapshot(&self) -> Arc<PolicySnapshot> {
        let guard = self.state.lock().await;
        Arc::new(guard.snapshot.clone())
    }

    async fn reload(&self) -> Result<(), PolicyError> {
        Err(PolicyError::NotImplemented("reload".into()))
    }

    async fn apply_override(&self, override_spec: RuntimeOverrideSpec) -> Result<(), PolicyError> {
        let ttl = if override_spec.ttl_seconds > 0 {
            Some(Duration::from_secs(override_spec.ttl_seconds))
        } else {
            None
        };
        let mut guard = self.state.lock().await;
        guard
            .overrides
            .insert(override_spec.path.clone(), override_spec.value.clone(), ttl);
        guard.apply_active_overrides()?;
        let snapshot = Arc::new(guard.snapshot.clone());
        drop(guard);

        let _ = self.watch_tx.send(snapshot.clone());

        if let Some(ttl) = ttl {
            let state = Arc::clone(&self.state);
            let watch_tx = self.watch_tx.clone();
            let path = override_spec.path.clone();
            tokio::spawn(async move {
                sleep(ttl).await;
                let mut guard = state.lock().await;
                if guard.overrides.remove(&path) {
                    match guard.apply_active_overrides() {
                        Ok(()) => {
                            let snapshot = Arc::new(guard.snapshot.clone());
                            drop(guard);
                            if watch_tx.send(snapshot).is_err() {
                                warn!("policy override expiry broadcast had no listeners");
                            }
                            return;
                        }
                        Err(err) => {
                            warn!("policy override expiry recompute failed: {err}");
                        }
                    }
                }
            });
        }

        Ok(())
    }

    fn subscribe(&self) -> watch::Receiver<Arc<PolicySnapshot>> {
        self.watch_tx.subscribe()
    }

    async fn guard(&self) -> PolicyGuard {
        let snapshot = self.snapshot().await;
        PolicyGuard { snapshot }
    }
}

#[derive(Clone, Debug)]
pub struct PolicyGuard {
    snapshot: Arc<PolicySnapshot>,
}

impl PolicyGuard {
    pub fn revision(&self) -> u64 {
        self.snapshot.rev
    }

    pub fn snapshot(&self) -> Arc<PolicySnapshot> {
        Arc::clone(&self.snapshot)
    }
}

pub(crate) fn apply_override_to_snapshot(
    snapshot: &mut PolicySnapshot,
    path: &str,
    value: &Value,
    source: PolicySource,
) -> Result<(), PolicyError> {
    let changed = match path {
        "scheduler.limits.global_slots" => merge_usize(
            &mut snapshot.scheduler.limits.global_slots,
            to_usize(value)?,
            source,
        ),
        "scheduler.limits.per_task_limit" => merge_usize(
            &mut snapshot.scheduler.limits.per_task_limit,
            to_usize(value)?,
            source,
        ),
        "scheduler.limits.queue_capacity" => merge_usize(
            &mut snapshot.scheduler.limits.queue_capacity,
            to_usize(value)?,
            source,
        ),
        "scheduler.retry.max_attempts" => merge_u8(
            &mut snapshot.scheduler.retry.max_attempts,
            to_u8(value)?,
            source,
        ),
        "scheduler.retry.backoff_ms" => merge_u64(
            &mut snapshot.scheduler.retry.backoff_ms,
            to_u64(value)?,
            source,
        ),
        "scheduler.timeouts_ms.navigate" => merge_u64(
            &mut snapshot.scheduler.timeouts_ms.navigate,
            to_u64(value)?,
            source,
        ),
        "scheduler.timeouts_ms.click" => merge_u64(
            &mut snapshot.scheduler.timeouts_ms.click,
            to_u64(value)?,
            source,
        ),
        "scheduler.timeouts_ms.type_text" => merge_u64(
            &mut snapshot.scheduler.timeouts_ms.type_text,
            to_u64(value)?,
            source,
        ),
        "scheduler.timeouts_ms.wait" => merge_u64(
            &mut snapshot.scheduler.timeouts_ms.wait,
            to_u64(value)?,
            source,
        ),
        "scheduler.timeouts_ms.screenshot" => merge_u64(
            &mut snapshot.scheduler.timeouts_ms.screenshot,
            to_u64(value)?,
            source,
        ),
        "registry.allow_multiple_pages" => {
            merge_bool(&mut snapshot.registry.allow_multiple_pages, to_bool(value)?)
        }
        "registry.health_probe_interval_ms" => merge_u64(
            &mut snapshot.registry.health_probe_interval_ms,
            to_u64(value)?,
            source,
        ),
        "features.state_center_persistence" => merge_bool(
            &mut snapshot.features.state_center_persistence,
            to_bool(value)?,
        ),
        "features.metrics_export" => {
            merge_bool(&mut snapshot.features.metrics_export, to_bool(value)?)
        }
        "features.registry_ingest_bus" => {
            merge_bool(&mut snapshot.features.registry_ingest_bus, to_bool(value)?)
        }
        path => return Err(PolicyError::UnsupportedPath(path.to_string())),
    };
    if changed {
        record_provenance(snapshot, path, source);
    }
    Ok(())
}

fn merge_usize(target: &mut usize, candidate: usize, source: PolicySource) -> bool {
    let original = *target;
    if matches!(source, PolicySource::RuntimeOverride | PolicySource::Cli) {
        *target = candidate;
    } else {
        *target = (*target).min(candidate);
    }
    *target != original
}

fn merge_u64(target: &mut u64, candidate: u64, source: PolicySource) -> bool {
    let original = *target;
    if matches!(source, PolicySource::RuntimeOverride | PolicySource::Cli) {
        *target = candidate;
    } else {
        *target = (*target).min(candidate);
    }
    *target != original
}

fn merge_u8(target: &mut u8, candidate: u8, source: PolicySource) -> bool {
    let original = *target;
    if matches!(source, PolicySource::RuntimeOverride | PolicySource::Cli) {
        *target = candidate;
    } else {
        *target = (*target).min(candidate);
    }
    *target != original
}

fn merge_bool(target: &mut bool, candidate: bool) -> bool {
    let original = *target;
    *target = candidate;
    *target != original
}

fn record_provenance(snapshot: &mut PolicySnapshot, path: &str, source: PolicySource) {
    snapshot.set_provenance(path, source);
}

fn to_usize(value: &Value) -> Result<usize, PolicyError> {
    value
        .as_u64()
        .or_else(|| value.as_i64().map(|v| v as u64))
        .map(|v| v as usize)
        .ok_or_else(|| PolicyError::InvalidValue(format!("expected integer, got {value}")))
}

fn to_u8(value: &Value) -> Result<u8, PolicyError> {
    value
        .as_u64()
        .or_else(|| value.as_i64().map(|v| v as u64))
        .ok_or_else(|| PolicyError::InvalidValue(format!("expected integer, got {value}")))
        .and_then(|v| {
            if v <= u8::MAX as u64 {
                Ok(v as u8)
            } else {
                Err(PolicyError::InvalidValue(format!("value {v} exceeds u8")))
            }
        })
}

fn to_u64(value: &Value) -> Result<u64, PolicyError> {
    value
        .as_u64()
        .or_else(|| value.as_i64().map(|v| v as u64))
        .ok_or_else(|| PolicyError::InvalidValue(format!("expected integer, got {value}")))
}

fn to_bool(value: &Value) -> Result<bool, PolicyError> {
    value
        .as_bool()
        .ok_or_else(|| PolicyError::InvalidValue(format!("expected bool, got {value}")))
}
