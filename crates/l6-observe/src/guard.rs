use crate::policy::current_policy;
use l6_privacy::{sanitize_labels as privacy_sanitize, PrivacyLabelMap};
use std::collections::BTreeMap;

pub type LabelMap = BTreeMap<String, String>;

pub fn sanitize_labels(kv: LabelMap) -> LabelMap {
    let policy = current_policy();
    let original = kv.clone();
    let mut sanitized: PrivacyLabelMap = privacy_sanitize(kv);

    if policy.allow_origin_full {
        if let Some(origin) = original.get("origin").cloned() {
            sanitized.insert("origin".into(), origin);
        }
    }

    if sanitized.len() > policy.series_limit {
        sanitized = sanitized.into_iter().take(policy.series_limit).collect();
    }

    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;
    use l6_privacy::policy::{set_policy as set_privacy_policy, PrivacyPolicyView};

    fn enable_privacy() {
        let mut view = PrivacyPolicyView::default();
        view.enable = true;
        set_privacy_policy(view);
    }

    #[test]
    fn test_host_only() {
        enable_privacy();
        let mut labels = LabelMap::new();
        labels.insert("origin".into(), "https://example.com/path".into());
        let sanitized = sanitize_labels(labels);
        assert_eq!(sanitized.get("origin").unwrap(), "example.com");
    }

    #[test]
    fn test_redact_pii() {
        enable_privacy();
        let mut labels = LabelMap::new();
        labels.insert("user".into(), "user@example.com".into());
        labels.insert("note".into(), "user@example.com".into());
        let sanitized = sanitize_labels(labels);
        assert!(!sanitized.contains_key("user"));
        let note = sanitized.get("note").unwrap();
        assert!(note.starts_with("sha256:"));
        assert!(!note.contains("user@example.com"));
    }
}
