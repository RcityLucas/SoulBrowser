use blake3::Hasher;
use serde_json::Value;
use zstd::stream::encode_all;

use crate::model::{DomAxRaw, StructMeta};
use crate::policy::StructCfg;

/// Applies sanitisation (mask + whitelist) and optional compression.
pub fn encode(domax: &DomAxRaw, cfg: &StructCfg) -> (Vec<u8>, Vec<u8>, StructMeta) {
    let sanitized_dom = mask_value(domax.dom.clone(), cfg);
    let sanitized_ax = mask_value(domax.ax.clone(), cfg);
    let dom_bytes = serde_json::to_vec(&sanitized_dom).unwrap_or_default();
    let ax_bytes = serde_json::to_vec(&sanitized_ax).unwrap_or_default();
    let dom_output = maybe_compress(&dom_bytes, cfg.compress, cfg.compress_level);
    let ax_output = maybe_compress(&ax_bytes, cfg.compress, cfg.compress_level);

    let mut meta = StructMeta::default();
    meta.bytes = (dom_output.len() + ax_output.len()) as u64;
    meta.masked = cfg.mask_text || !cfg.mask_secret_fields.is_empty();
    meta.node_count = sanitized_dom
        .as_array()
        .map(|arr| arr.len())
        .unwrap_or_default() as u32;
    meta.masked_fields = collect_masked_fields(&sanitized_dom);
    (dom_output, ax_output, meta)
}

pub fn content_hash(dom: &[u8], ax: &[u8]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(dom);
    hasher.update(ax);
    format!("ss_{}", hasher.finalize().to_hex())
}

fn maybe_compress(src: &[u8], compress: bool, level: i32) -> Vec<u8> {
    if !compress || src.is_empty() {
        return src.to_vec();
    }
    encode_all(src, level.max(0)).unwrap_or_else(|_| src.to_vec())
}

fn mask_value(mut value: Value, cfg: &StructCfg) -> Value {
    if cfg.field_whitelist.is_empty() {
        return value;
    }
    if let Value::Object(ref mut map) = value {
        map.retain(|k, _| cfg.field_whitelist.iter().any(|f| f == k));
        for (key, val) in map.iter_mut() {
            if cfg
                .mask_secret_fields
                .iter()
                .any(|keyword| key.to_lowercase().contains(keyword))
            {
                *val = Value::String("***".into());
            } else {
                mask_text_field(val, cfg);
            }
        }
    }
    value
}

fn mask_text_field(value: &mut Value, cfg: &StructCfg) {
    match value {
        Value::String(s) => {
            if s.len() > cfg.max_text_len {
                s.truncate(cfg.max_text_len);
            }
            if cfg
                .mask_secret_fields
                .iter()
                .any(|keyword| s.to_lowercase().contains(keyword))
            {
                *s = "***".into();
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                mask_text_field(v, cfg);
            }
        }
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if cfg
                    .mask_secret_fields
                    .iter()
                    .any(|keyword| key.to_lowercase().contains(keyword))
                {
                    *v = Value::String("***".into());
                } else {
                    mask_text_field(v, cfg);
                }
            }
        }
        _ => {}
    }
}

fn collect_masked_fields(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Value::Object(map) = value {
        for (key, val) in map {
            if matches!(val, Value::String(s) if s == "***") {
                out.push(key.clone());
            }
        }
    }
    out
}
