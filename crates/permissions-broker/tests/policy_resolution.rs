use permissions_broker::{
    Broker, DecisionKind, PermissionsBroker, PolicyFile, PolicyTemplate, SitePolicy,
};

fn sample_policy(entries: Vec<SitePolicy>) -> PolicyFile {
    PolicyFile {
        version: 1,
        defaults: PolicyTemplate {
            allow: vec!["clipboard_read".into()],
            deny: vec![],
            ttl: Some("session".into()),
        },
        sites: entries,
    }
}

#[tokio::test]
async fn default_policy_allows_clipboard_read() {
    let broker = PermissionsBroker::new();
    broker.load_policy(sample_policy(vec![])).await;
    let decision = broker.apply_policy("https://example.com").await.unwrap();
    assert_eq!(decision.kind, DecisionKind::Allow);
    assert_eq!(decision.allowed, vec!["clipboard_read".to_string()]);
    assert!(decision.ttl_ms.is_none()); // session ttl maps to None
}

#[tokio::test]
async fn site_override_applies_allow_and_deny() {
    let broker = PermissionsBroker::new();
    broker
        .load_policy(sample_policy(vec![SitePolicy {
            match_pattern: "https://pay.example.com".into(),
            allow: Some(vec!["clipboard_write".into(), "download".into()]),
            deny: Some(vec!["clipboard_read".into()]),
            ttl: Some("30m".into()),
            notes: None,
        }]))
        .await;

    let decision = broker
        .apply_policy("https://pay.example.com")
        .await
        .unwrap();
    assert_eq!(decision.kind, DecisionKind::Partial);
    assert_eq!(
        decision.allowed,
        vec!["clipboard_write".to_string(), "download".to_string()]
    );
    assert!(decision.denied.contains(&"clipboard_read".to_string()));
    assert!(decision.ttl_ms.unwrap() >= 30 * 60 * 1000);
}

#[tokio::test]
async fn ensure_for_filters_needs() {
    let broker = PermissionsBroker::new();
    broker
        .load_policy(sample_policy(vec![SitePolicy {
            match_pattern: "https://docs.example.com".into(),
            allow: Some(vec!["download".into(), "clipboard_read".into()]),
            deny: None,
            ttl: None,
            notes: None,
        }]))
        .await;

    let decision = broker
        .ensure_for("https://docs.example.com", &vec!["download".into()])
        .await
        .unwrap();

    assert_eq!(decision.allowed, vec!["download".to_string()]);
    assert!(decision.missing.is_empty());
}

#[tokio::test]
async fn ensure_for_reports_missing_permissions() {
    let broker = PermissionsBroker::new();
    broker.load_policy(sample_policy(vec![])).await;

    let decision = broker
        .ensure_for("https://api.example.com", &vec!["download".into()])
        .await
        .unwrap();

    assert_eq!(decision.kind, DecisionKind::Deny);
    assert_eq!(decision.missing, vec!["download".to_string()]);
}

#[tokio::test]
async fn missing_policy_yields_error() {
    let broker = PermissionsBroker::new();
    let result = broker.apply_policy("https://no-policy.example.com").await;
    assert!(result.is_err());
}
