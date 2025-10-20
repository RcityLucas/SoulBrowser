//! Text redaction helpers.
//!
//! P0：提供简单的占位逻辑，后续引入策略化脱敏。

use serde_json::Value;

pub fn redact_value(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_text(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(redact_value).collect()),
        Value::Object(map) => {
            let mut redacted = serde_json::Map::with_capacity(map.len());
            for (key, val) in map {
                redacted.insert(key, redact_value(val));
            }
            Value::Object(redacted)
        }
        other => other,
    }
}

pub fn redact_text(text: &str) -> String {
    if text.len() <= 64 {
        return text.to_string();
    }
    let mut truncated = text[..64].to_string();
    truncated.push_str("…");
    truncated
}
