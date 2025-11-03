use permissions_broker::{
    Broker, BrokerError, DecisionKind, PermissionMap, PermissionTransport, PermissionsBroker,
    PolicyFile, PolicyTemplate, SitePolicy,
};
use std::sync::Arc;
use tokio::sync::Mutex;

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
    broker.load_policy(sample_policy(vec![])).await.unwrap();
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
        .await
        .unwrap();

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
        .await
        .unwrap();

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
    broker.load_policy(sample_policy(vec![])).await.unwrap();

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

#[tokio::test]
async fn audit_event_emitted_on_decision() {
    let broker = PermissionsBroker::new();
    broker.load_policy(sample_policy(vec![])).await.unwrap();
    let mut rx = broker.subscribe();

    let _ = broker
        .apply_policy("https://events.example.com")
        .await
        .unwrap();

    let event = rx.recv().await.expect("receive audit event");
    assert_eq!(event.origin, "https://events.example.com");
    assert_eq!(event.decision, DecisionKind::Allow);
}

#[tokio::test]
async fn permission_whitelist_rejects_unknown_entries() {
    let broker = PermissionsBroker::new();
    let mut map = PermissionMap::new();
    map.insert("clipboard_read".into(), "clipboard-read".into());
    broker.set_permission_map(map).await;

    let result = broker
        .load_policy(sample_policy(vec![SitePolicy {
            match_pattern: "https://bad.example.com".into(),
            allow: Some(vec!["unknown".into()]),
            deny: None,
            ttl: None,
            notes: None,
        }]))
        .await;

    assert!(matches!(result, Err(BrokerError::Internal(_))));
}

struct RecordingTransport {
    calls: Arc<Mutex<Vec<(String, Vec<String>, Vec<String>)>>>,
}

impl Default for RecordingTransport {
    fn default() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Clone for RecordingTransport {
    fn clone(&self) -> Self {
        Self {
            calls: Arc::clone(&self.calls),
        }
    }
}

#[async_trait::async_trait]
impl PermissionTransport for RecordingTransport {
    async fn apply_permissions(
        &self,
        origin: &str,
        grant: &[String],
        revoke: &[String],
    ) -> Result<(), BrokerError> {
        let mut guard = self.calls.lock().await;
        guard.push((origin.to_string(), grant.to_vec(), revoke.to_vec()));
        Ok(())
    }
}

#[tokio::test]
async fn transport_receives_translated_permissions() {
    let broker = PermissionsBroker::new();
    let mut map = PermissionMap::new();
    map.insert("clipboard_read".into(), "clipboardRead".into());
    broker.set_permission_map(map).await;
    broker.load_policy(sample_policy(vec![])).await.unwrap();
    let transport = RecordingTransport::default();
    broker.set_transport(Arc::new(transport.clone())).await;

    broker.apply_policy("https://example.com").await.unwrap();

    let calls = transport.calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "https://example.com");
    assert_eq!(calls[0].1, vec!["clipboardRead".to_string()]);
    assert!(calls[0].2.is_empty());
}

#[tokio::test]
async fn transport_revoke_includes_denied_and_missing() {
    let broker = PermissionsBroker::new();
    let mut map = PermissionMap::new();
    map.insert("clipboard_read".into(), "clipboardRead".into());
    map.insert("camera".into(), "camera".into());
    broker.set_permission_map(map).await;
    broker
        .load_policy(sample_policy(vec![SitePolicy {
            match_pattern: "https://deny.example.com".into(),
            allow: Some(vec!["clipboard_read".into()]),
            deny: Some(vec!["camera".into()]),
            ttl: None,
            notes: None,
        }]))
        .await
        .unwrap();
    let transport = RecordingTransport::default();
    broker.set_transport(Arc::new(transport.clone())).await;

    broker
        .ensure_for("https://deny.example.com", &vec!["camera".into()])
        .await
        .unwrap();

    let calls = transport.calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1.is_empty());
    assert_eq!(calls[0].2, vec!["camera".to_string()]);
}
