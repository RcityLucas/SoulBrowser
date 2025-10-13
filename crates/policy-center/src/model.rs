use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PolicySnapshot {
    pub rev: u64,
    pub scheduler: SchedulerPolicy,
    pub registry: RegistryPolicy,
    pub features: FeatureFlags,
    pub provenance: HashMap<String, PolicyProvenance>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SchedulerPolicy {
    pub limits: SchedulerLimits,
    pub timeouts_ms: SchedulerTimeouts,
    pub retry: RetryPolicy,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SchedulerLimits {
    pub global_slots: usize,
    pub per_task_limit: usize,
    pub queue_capacity: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SchedulerTimeouts {
    pub navigate: u64,
    pub click: u64,
    pub type_text: u64,
    pub wait: u64,
    pub screenshot: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct RetryPolicy {
    pub max_attempts: u8,
    pub backoff_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct RegistryPolicy {
    pub health_probe_interval_ms: u64,
    pub allow_multiple_pages: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct FeatureFlags {
    pub state_center_persistence: bool,
    pub metrics_export: bool,
    pub registry_ingest_bus: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyProvenance {
    pub path: String,
    pub source: PolicySource,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PolicySource {
    Builtin,
    File,
    Env,
    Cli,
    RuntimeOverride,
}

#[derive(Clone, Debug)]
pub struct PolicyView {
    pub rev: u64,
    pub scheduler: SchedulerPolicy,
    pub registry: RegistryPolicy,
    pub features: FeatureFlags,
}

impl From<PolicySnapshot> for PolicyView {
    fn from(snapshot: PolicySnapshot) -> Self {
        Self {
            rev: snapshot.rev,
            scheduler: snapshot.scheduler,
            registry: snapshot.registry,
            features: snapshot.features,
        }
    }
}

impl PolicySnapshot {
    pub fn set_provenance(&mut self, path: &str, source: PolicySource) {
        self.provenance.insert(
            path.to_string(),
            PolicyProvenance {
                path: path.to_string(),
                source,
            },
        );
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeOverrideSpec {
    pub path: String,
    pub value: serde_json::Value,
    pub owner: String,
    pub reason: String,
    pub ttl_seconds: u64,
}
