use crate::model::{
    FeatureFlags, PerceiverPolicies, PolicySnapshot, RegistryPolicy, RetryPolicy, SchedulerLimits,
    SchedulerPolicy, SchedulerTimeouts, StructuralCachePolicy, StructuralDiffPolicy,
    StructuralJudgePolicy, StructuralPerceiverPolicy, StructuralResolvePolicy,
    StructuralScoreWeights,
};

pub fn default_snapshot() -> PolicySnapshot {
    PolicySnapshot {
        rev: 1,
        scheduler: SchedulerPolicy {
            limits: SchedulerLimits {
                global_slots: 8,
                per_task_limit: 3,
                queue_capacity: 1024,
            },
            timeouts_ms: SchedulerTimeouts {
                navigate: 15_000,
                click: 5_000,
                type_text: 5_000,
                wait: 10_000,
                screenshot: 10_000,
            },
            retry: RetryPolicy {
                max_attempts: 1,
                backoff_ms: 300,
            },
        },
        registry: RegistryPolicy {
            health_probe_interval_ms: 5_000,
            allow_multiple_pages: true,
        },
        features: FeatureFlags {
            state_center_persistence: false,
            metrics_export: false,
            registry_ingest_bus: false,
        },
        perceiver: PerceiverPolicies {
            structural: StructuralPerceiverPolicy {
                resolve: StructuralResolvePolicy {
                    max_candidates: 1,
                    fuzziness: None,
                    debounce_ms: Some(250),
                },
                weights: StructuralScoreWeights {
                    visibility: 0.05,
                    accessibility: 0.06,
                    text: 0.05,
                    geometry: 0.1,
                    backend: 0.25,
                },
                judge: StructuralJudgePolicy {
                    minimum_opacity: None,
                    minimum_visible_area: None,
                    pointer_events_block: true,
                },
                diff: StructuralDiffPolicy {
                    debounce_ms: None,
                    max_changes: None,
                    focus: None,
                },
                cache: StructuralCachePolicy {
                    anchor_ttl_ms: 250,
                    snapshot_ttl_ms: 1_000,
                },
            },
        },
        provenance: Default::default(),
    }
}
