use std::collections::HashMap;

use serde_json::Value;

use crate::model::DomAxRaw;

pub type FeatureMap = HashMap<String, f32>;

/// Extracts lightweight features from DOM/AX payloads.
pub fn extract_domax(
    domax: &DomAxRaw,
    whitelist: &[String],
    mask_text: bool,
    max_text_len: usize,
) -> FeatureMap {
    let mut feats = FeatureMap::new();
    collect_dom(&domax.dom, whitelist, mask_text, max_text_len, &mut feats);
    collect_dom(&domax.ax, whitelist, mask_text, max_text_len, &mut feats);
    feats
}

fn collect_dom(
    value: &Value,
    whitelist: &[String],
    mask_text: bool,
    max_text_len: usize,
    feats: &mut FeatureMap,
) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if whitelist.iter().any(|w| w == k) {
                    let feature_key = format!("field::{k}");
                    feats
                        .entry(feature_key)
                        .and_modify(|w| *w += 1.0)
                        .or_insert(1.0);
                }
                collect_dom(v, whitelist, mask_text, max_text_len, feats);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                collect_dom(v, whitelist, mask_text, max_text_len, feats);
            }
        }
        Value::String(text) => {
            if !mask_text {
                let truncated = text.chars().take(max_text_len).collect::<String>();
                feats
                    .entry(format!("text::{truncated}"))
                    .and_modify(|w| *w += 1.0)
                    .or_insert(1.0);
            }
        }
        _ => {}
    }
}
