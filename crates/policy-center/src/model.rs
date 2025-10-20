use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PolicySnapshot {
    pub rev: u64,
    pub scheduler: SchedulerPolicy,
    pub registry: RegistryPolicy,
    pub features: FeatureFlags,
    #[serde(default)]
    pub perceiver: PerceiverPolicies,
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

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PerceiverPolicies {
    pub structural: StructuralPerceiverPolicy,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StructuralPerceiverPolicy {
    pub resolve: StructuralResolvePolicy,
    pub weights: StructuralScoreWeights,
    pub judge: StructuralJudgePolicy,
    pub diff: StructuralDiffPolicy,
    pub cache: StructuralCachePolicy,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StructuralResolvePolicy {
    pub max_candidates: usize,
    pub fuzziness: Option<f32>,
    pub debounce_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StructuralScoreWeights {
    pub visibility: f32,
    pub accessibility: f32,
    pub text: f32,
    pub geometry: f32,
    pub backend: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StructuralJudgePolicy {
    pub minimum_opacity: Option<f32>,
    pub minimum_visible_area: Option<f64>,
    pub pointer_events_block: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StructuralDiffPolicy {
    pub debounce_ms: Option<u64>,
    pub max_changes: Option<usize>,
    pub focus: Option<StructuralDiffFocus>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StructuralDiffFocus {
    pub backend_node_id: Option<u64>,
    pub geometry: Option<StructuralDiffGeometry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StructuralDiffGeometry {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StructuralCachePolicy {
    pub anchor_ttl_ms: u64,
    pub snapshot_ttl_ms: u64,
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
    pub perceiver: PerceiverPolicies,
}

impl From<PolicySnapshot> for PolicyView {
    fn from(snapshot: PolicySnapshot) -> Self {
        Self {
            rev: snapshot.rev,
            scheduler: snapshot.scheduler,
            registry: snapshot.registry,
            features: snapshot.features,
            perceiver: snapshot.perceiver,
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
