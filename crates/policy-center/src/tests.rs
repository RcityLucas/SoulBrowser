use crate::api::{InMemoryPolicyCenter, PolicyCenter};
use crate::defaults::default_snapshot;
use crate::loader::load_snapshot;
use crate::model::RuntimeOverrideSpec;
use std::env;
use std::sync::{Arc, Mutex, OnceLock};

#[test]
fn default_snapshot_has_reasonable_limits() {
    let snapshot = default_snapshot();
    assert_eq!(snapshot.scheduler.limits.global_slots, 8);
    assert_eq!(snapshot.scheduler.retry.max_attempts, 1);
}

#[test]
fn load_snapshot_allows_override() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("policy.yaml");
    std::fs::write(
        &file_path,
        r#"scheduler:
  limits:
    global_slots: 4
    per_task_limit: 2
    queue_capacity: 100
  timeouts_ms:
    navigate: 1000
    click: 2000
    type_text: 2000
    wait: 3000
    screenshot: 4000
  retry:
    max_attempts: 2
    backoff_ms: 100
"#,
    )
    .unwrap();

    let snapshot = load_snapshot(Some(&file_path)).unwrap();
    assert_eq!(snapshot.scheduler.limits.global_slots, 4);
    assert_eq!(snapshot.scheduler.retry.max_attempts, 1);
}

#[tokio::test]
async fn override_updates_snapshot() {
    let center = InMemoryPolicyCenter::new(default_snapshot());
    let spec = RuntimeOverrideSpec {
        path: "scheduler.limits.global_slots".into(),
        value: serde_json::json!(4),
        owner: "test".into(),
        reason: "unit test".into(),
        ttl_seconds: 0,
    };
    PolicyCenter::apply_override(&center, spec).await.unwrap();
    let snapshot = PolicyCenter::snapshot(&center).await;
    assert_eq!(snapshot.scheduler.limits.global_slots, 4);
}

#[tokio::test]
async fn subscribe_streams_updates() {
    let center = InMemoryPolicyCenter::new(default_snapshot());
    let mut rx = PolicyCenter::subscribe(&center);
    let original_rev = rx.borrow().rev;

    let spec = RuntimeOverrideSpec {
        path: "registry.health_probe_interval_ms".into(),
        value: serde_json::json!(2500),
        owner: "test".into(),
        reason: "unit test".into(),
        ttl_seconds: 0,
    };
    PolicyCenter::apply_override(&center, spec).await.unwrap();
    rx.changed().await.unwrap();
    let snapshot = Arc::clone(&rx.borrow());
    assert_ne!((*snapshot).rev, original_rev);
    assert_eq!((*snapshot).registry.health_probe_interval_ms, 2500);
}

#[tokio::test]
async fn guard_provides_sticky_view() {
    let center = InMemoryPolicyCenter::new(default_snapshot());
    let guard = center.guard().await;
    let snapshot = guard.snapshot();
    assert_eq!(guard.revision(), snapshot.rev);
}

#[test]
fn env_cascade_prefers_stricter_value() {
    let _guard = env_guard().lock().unwrap();
    let key = "SOUL_POLICY__SCHEDULER__LIMITS__GLOBAL_SLOTS";
    env::set_var(key, "4");
    let snapshot = load_snapshot(None).expect("load snapshot");
    env::remove_var(key);
    assert_eq!(snapshot.scheduler.limits.global_slots, 4);
    assert_eq!(
        snapshot
            .provenance
            .get("scheduler.limits.global_slots")
            .expect("provenance")
            .source,
        crate::model::PolicySource::Env
    );
}

#[test]
fn cli_overrides_replace_and_record_provenance() {
    let _guard = env_guard().lock().unwrap();
    env::set_var(
        "SOUL_POLICY_CLI_OVERRIDES",
        "features.state_center_persistence=true,scheduler.retry.max_attempts=3",
    );
    let snapshot = load_snapshot(None).expect("load snapshot with cli");
    env::remove_var("SOUL_POLICY_CLI_OVERRIDES");
    assert!(snapshot.features.state_center_persistence);
    assert_eq!(snapshot.scheduler.retry.max_attempts, 3);
    assert_eq!(
        snapshot
            .provenance
            .get("features.state_center_persistence")
            .unwrap()
            .source,
        crate::model::PolicySource::Cli
    );
}

fn env_guard() -> &'static Mutex<()> {
    static ENV_GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_GUARD.get_or_init(|| Mutex::new(()))
}
