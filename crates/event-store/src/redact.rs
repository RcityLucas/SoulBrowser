use crate::config::RedactRules;
use crate::model::EventEnvelope;

/// Applies conservative redaction rules to an event payload.
pub fn apply(envelope: &mut EventEnvelope, rules: &RedactRules, max_payload_bytes: usize) {
    if let Some(obj) = envelope.payload.as_object_mut() {
        if rules.mask_url_query {
            mask_url_fields(obj);
        }
        for key in rules.forbid_keys() {
            obj.remove(key);
        }
    }

    if let Ok(mut raw) = serde_json::to_vec(&envelope.payload) {
        if raw.len() > max_payload_bytes {
            raw.truncate(max_payload_bytes);
            if let Ok(truncated) = serde_json::from_slice(&raw) {
                envelope.payload = truncated;
            }
        }
    }
}

fn mask_url_fields(map: &mut serde_json::Map<String, serde_json::Value>) {
    for value in map.values_mut() {
        if let Some(url) = value.as_str() {
            if let Some((path, _)) = url.split_once('?') {
                *value = serde_json::Value::String(format!("{path}?***"));
            }
        } else if let Some(obj) = value.as_object_mut() {
            mask_url_fields(obj);
        }
    }
}

trait RedactRuleExt {
    fn forbid_keys(&self) -> &[String];
}

impl RedactRuleExt for RedactRules {
    fn forbid_keys(&self) -> &[String] {
        // Placeholder for future expansion.
        &[]
    }
}
