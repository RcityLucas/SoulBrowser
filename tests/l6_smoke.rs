use l6_observe::guard::LabelMap as ObserveLabelMap;
use l6_observe::metrics;
use l6_observe::policy::{set_policy as set_observe_policy, ObsPolicyView};
use l6_privacy::apply::apply_export;
use l6_privacy::policy::{set_policy as set_privacy_policy, PrivacyPolicyView};
use l6_privacy::{RedactCtx, RedactScope};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::json;

static L6_TEST_GUARD: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[test]
fn privacy_and_observe_metrics_integration() {
    let _guard = L6_TEST_GUARD.lock();

    let mut privacy = PrivacyPolicyView::default();
    privacy.enable = true;
    set_privacy_policy(privacy);

    let mut observe = ObsPolicyView::default();
    observe.enable_metrics = true;
    set_observe_policy(observe);

    metrics::ensure_metrics();
    let mut labels = ObserveLabelMap::new();
    labels.insert("origin".into(), "https://example.com/path?q=1".into());
    labels.insert("note".into(), "user@example.com".into());
    labels.insert("user".into(), "user@example.com".into());

    metrics::inc("l6_privacy_integration_counter", labels);

    let rendered = metrics::render_prometheus();
    let line = rendered
        .lines()
        .find(|line| line.starts_with("l6_privacy_integration_counter"))
        .expect("metric line present");

    assert!(line.contains("origin=\"example.com\""));
    assert!(line.contains("note=\"sha256:"));
    assert!(!line.contains("user="));
    assert!(!line.contains("user@example.com"));
}

#[test]
fn privacy_export_redaction_smoke() {
    let _guard = L6_TEST_GUARD.lock();

    let mut privacy = PrivacyPolicyView::default();
    privacy.enable = true;
    privacy.query_allow_keys = vec!["safe".into()];
    set_privacy_policy(privacy);

    let ctx = RedactCtx {
        scope: RedactScope::Export,
        export: true,
        ..Default::default()
    };

    let mut line = json!({
        "payload": {
            "message": "contact me at foo@example.com",
        },
        "target_url": "https://example.com/search?q=secret&safe=1"
    });

    let report = apply_export(&mut line, &ctx).expect("redaction succeeds");
    assert!(report.applied);

    let message = line["payload"]["message"].clone();
    let hash = message["hash"].as_str().expect("hash present");
    assert!(hash.starts_with("sha256:"));
    let expected_len = "contact me at ***".len() as u64;
    assert_eq!(message["len"].as_u64(), Some(expected_len));

    let url = line["target_url"].as_str().expect("url remains string");
    assert_eq!(url, "https://example.com/search?q=***&safe=1");
}
