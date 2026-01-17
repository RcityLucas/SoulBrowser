use crate::policy::current_policy;
use crate::text::{digest, mask_pii};
use std::collections::BTreeMap;
use url::Url;

pub type LabelMap = BTreeMap<String, String>;

pub fn sanitize_labels(mut labels: LabelMap) -> LabelMap {
    let policy = current_policy();

    if policy.origin_host_only {
        if let Some(origin) = labels.get_mut("origin") {
            if let Ok(parsed) = Url::parse(origin) {
                if let Some(host) = parsed.host_str() {
                    *origin = host.to_string();
                }
            } else if let Some((host, _rest)) = origin.split_once('/') {
                *origin = host.to_string();
            }
        }
    }

    labels.retain(|key, _| !policy.ban_labels.iter().any(|ban| matches_key(ban, key)));

    for (key, value) in labels.iter_mut() {
        if key.ends_with("_url") || key == "href" || key == "src" {
            *value = crate::url::redact_url(value, &policy.query_allow_keys);
        } else {
            let masked = mask_pii(value, &policy.pii_patterns);
            if masked != *value {
                let (hash, _len) = digest(
                    &masked,
                    policy.text_hash_alg.clone(),
                    policy.message_max_len,
                );
                *value = hash;
            }
        }

        if value.len() > policy.label_max_len {
            value.truncate(policy.label_max_len);
        }
    }

    labels
}

fn matches_key(pattern: &str, key: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        key.starts_with(prefix)
    } else {
        pattern == key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{set_policy, PrivacyPolicyView};

    fn enable_policy() {
        let mut view = PrivacyPolicyView::default();
        view.enable = true;
        set_policy(view);
    }

    #[test]
    fn sanitizes_origin() {
        enable_policy();
        let mut labels = LabelMap::new();
        labels.insert("origin".into(), "https://example.com/path?q=1".into());
        let out = sanitize_labels(labels);
        assert_eq!(out.get("origin").unwrap(), "example.com");
    }

    #[test]
    fn masks_pii() {
        enable_policy();
        let mut labels = LabelMap::new();
        labels.insert("user".into(), "user@example.com".into());
        labels.insert("note".into(), "user@example.com".into());
        let out = sanitize_labels(labels);
        assert!(!out.contains_key("user"));
        let value = out.get("note").unwrap();
        assert!(value.starts_with("sha256:"));
        assert!(!value.contains("user@example.com"));
    }
}
