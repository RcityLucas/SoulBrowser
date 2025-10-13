use crate::model::{
    FeatureFlags, PolicySnapshot, RegistryPolicy, RetryPolicy, SchedulerLimits, SchedulerPolicy,
    SchedulerTimeouts,
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
        provenance: Default::default(),
    }
}
