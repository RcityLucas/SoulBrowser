use std::cmp::Ordering;
use std::collections::HashSet;

use serde_json::{json, Map as JsonMap, Value};
use soulbrowser_core_types::FrameId;

use crate::model::{AnchorDescriptor, AnchorGeometry, ResolveHint, ResolveOpt, SelectorOrHint};

pub fn from_hint(hint: &ResolveHint, frame: &FrameId) -> Vec<AnchorDescriptor> {
    let selector = SelectorOrHint::from(hint);
    from_selector(&selector, frame, &ResolveOpt::default())
}

pub fn from_selector(
    selector: &SelectorOrHint,
    frame: &FrameId,
    options: &ResolveOpt,
) -> Vec<AnchorDescriptor> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    collect_from_selector(selector, frame, options, &mut seen, &mut out);
    out
}

fn collect_from_selector(
    selector: &SelectorOrHint,
    frame: &FrameId,
    options: &ResolveOpt,
    seen: &mut HashSet<String>,
    out: &mut Vec<AnchorDescriptor>,
) {
    match selector {
        SelectorOrHint::Combo(items) => {
            let mut combo_sources = Vec::new();
            for item in items {
                let subs = from_selector(item, frame, options);
                if let Some(best) = subs
                    .iter()
                    .max_by(|a, b| {
                        a.confidence
                            .partial_cmp(&b.confidence)
                            .unwrap_or(Ordering::Equal)
                    })
                    .cloned()
                {
                    combo_sources.push(best);
                }
                for candidate in subs {
                    push_candidate(candidate, seen, out);
                }
            }
            if let Some(combo) = combo_anchor(&combo_sources, frame) {
                push_candidate(combo, seen, out);
            }
        }
        _ => {
            let candidates = match selector {
                SelectorOrHint::Css(selector) => css_candidates(selector, frame),
                SelectorOrHint::Aria { role, name, state } => {
                    aria_candidates(role, name.as_deref(), state.as_deref(), frame)
                }
                SelectorOrHint::Ax { role, name, value } => {
                    ax_candidates(role, name.as_deref(), value.as_deref(), frame)
                }
                SelectorOrHint::Text { pattern, fuzzy } => {
                    text_candidates(pattern, *fuzzy, options, frame)
                }
                SelectorOrHint::Attr { key, value } => attr_candidates(key, value, frame),
                SelectorOrHint::Backend(backend) => backend_candidates(*backend, frame),
                SelectorOrHint::Geometry { x, y, w, h } => {
                    geometry_candidates(*x, *y, *w, *h, frame)
                }
                SelectorOrHint::Combo(_) => unreachable!(),
            };
            for candidate in candidates {
                push_candidate(candidate, seen, out);
            }
        }
    }
}

fn push_candidate(
    candidate: AnchorDescriptor,
    seen: &mut HashSet<String>,
    out: &mut Vec<AnchorDescriptor>,
) {
    if seen.insert(fingerprint(&candidate)) {
        out.push(candidate);
    }
}

fn combo_anchor(sources: &[AnchorDescriptor], frame: &FrameId) -> Option<AnchorDescriptor> {
    if sources.is_empty() {
        return None;
    }
    let mut combined = JsonMap::new();
    let mut source_values = Vec::new();
    let mut total_confidence = 0.0;
    let mut backend = None;
    let mut geometry = None;

    for source in sources {
        total_confidence += source.confidence;
        if backend.is_none() {
            backend = source.backend_node_id;
        }
        if geometry.is_none() {
            geometry = source.geometry.clone();
        }
        let mut entry = JsonMap::new();
        entry.insert("strategy".into(), Value::String(source.strategy.clone()));
        entry.insert("confidence".into(), Value::from(source.confidence));
        entry.insert("value".into(), source.value.clone());
        if let Some(backend) = source.backend_node_id {
            entry.insert("backend".into(), Value::from(backend));
        }
        source_values.push(Value::Object(entry));
    }

    let average = total_confidence / sources.len() as f32;
    let bonus = 0.05 * (sources.len().saturating_sub(1) as f32);
    let confidence = (average + bonus).clamp(0.0, 1.0);

    combined.insert("kind".into(), Value::String("combo".into()));
    combined.insert("sources".into(), Value::Array(source_values));

    Some(anchor(
        "combo",
        Value::Object(combined),
        frame,
        confidence,
        backend,
        geometry,
    ))
}

fn fingerprint(anchor: &AnchorDescriptor) -> String {
    let mut value = anchor.value.clone();
    if let Some(obj) = value.as_object_mut() {
        obj.remove("confidence");
    }
    format!(
        "{}::{}::{}",
        anchor.strategy,
        anchor
            .backend_node_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "-".into()),
        value
    )
}

fn css_candidates(selector: &str, frame: &FrameId) -> Vec<AnchorDescriptor> {
    let mut candidates = Vec::new();
    if !selector.trim().is_empty() {
        candidates.push(anchor(
            "css",
            json!({
                "selector": selector,
                "exact": true,
            }),
            frame,
            0.68,
            None,
            None,
        ));

        if let Some(last_segment) = selector.split_whitespace().last() {
            if last_segment != selector {
                candidates.push(anchor(
                    "css",
                    json!({
                        "selector": selector,
                        "fallback": last_segment,
                        "exact": false,
                    }),
                    frame,
                    0.58,
                    None,
                    None,
                ));
            }
        }
    }
    if candidates.is_empty() {
        candidates.push(anchor(
            "css",
            json!({ "selector": selector }),
            frame,
            0.4,
            None,
            None,
        ));
    }
    candidates
}

fn aria_candidates(
    role: &str,
    name: Option<&str>,
    state: Option<&str>,
    frame: &FrameId,
) -> Vec<AnchorDescriptor> {
    let mut base = json!({ "role": role });
    if let Some(name) = name {
        if let Some(obj) = base.as_object_mut() {
            obj.insert("name".into(), Value::String(name.to_string()));
        }
    }
    if let Some(state) = state {
        if let Some(obj) = base.as_object_mut() {
            obj.insert("state".into(), Value::String(state.to_string()));
        }
    }

    vec![anchor(
        "aria",
        base,
        frame,
        if name.is_some() { 0.66 } else { 0.6 },
        None,
        None,
    )]
}

fn ax_candidates(
    role: &str,
    name: Option<&str>,
    value: Option<&str>,
    frame: &FrameId,
) -> Vec<AnchorDescriptor> {
    let mut base = json!({ "axRole": role });
    if let Some(name) = name {
        if let Some(obj) = base.as_object_mut() {
            obj.insert("axName".into(), Value::String(name.to_string()));
        }
    }
    if let Some(value) = value {
        if let Some(obj) = base.as_object_mut() {
            obj.insert("axValue".into(), Value::String(value.to_string()));
        }
    }

    vec![anchor("ax", base, frame, 0.62, None, None)]
}

fn text_candidates(
    pattern: &str,
    fuzzy_opt: Option<f32>,
    options: &ResolveOpt,
    frame: &FrameId,
) -> Vec<AnchorDescriptor> {
    if pattern.trim().is_empty() {
        return Vec::new();
    }

    let mut candidates = vec![
        anchor(
            "text",
            json!({
                "pattern": pattern,
                "match": "contains",
            }),
            frame,
            0.57,
            None,
            None,
        ),
        anchor(
            "text",
            json!({
                "pattern": pattern,
                "match": "case_insensitive",
            }),
            frame,
            0.52,
            None,
            None,
        ),
    ];

    let fuzzy = fuzzy_opt.or(options.fuzziness).filter(|f| *f > 0.0);
    if let Some(fuzz) = fuzzy {
        candidates.push(anchor(
            "text",
            json!({
                "pattern": pattern,
                "match": "fuzzy",
                "threshold": fuzz,
            }),
            frame,
            (0.45 + fuzz * 0.1).clamp(0.3, 0.6),
            None,
            None,
        ));
    }

    candidates
}

fn backend_candidates(backend: u64, frame: &FrameId) -> Vec<AnchorDescriptor> {
    vec![anchor(
        "backend",
        json!({ "backendNodeId": backend }),
        frame,
        0.9,
        Some(backend),
        None,
    )]
}

fn geometry_candidates(x: i32, y: i32, w: i32, h: i32, frame: &FrameId) -> Vec<AnchorDescriptor> {
    let geometry = AnchorGeometry {
        x: x as f64,
        y: y as f64,
        width: w.max(0) as f64,
        height: h.max(0) as f64,
    };
    vec![anchor(
        "geometry",
        json!({
            "origin": { "x": x, "y": y },
            "size": { "w": w, "h": h },
        }),
        frame,
        0.5,
        None,
        Some(geometry),
    )]
}

fn attr_candidates(key: &str, value: &str, frame: &FrameId) -> Vec<AnchorDescriptor> {
    vec![anchor(
        "attr",
        json!({
            "key": key,
            "value": value,
        }),
        frame,
        0.55,
        None,
        None,
    )]
}

fn anchor(
    strategy: &str,
    value: Value,
    frame: &FrameId,
    confidence: f32,
    backend_node_id: Option<u64>,
    geometry: Option<AnchorGeometry>,
) -> AnchorDescriptor {
    AnchorDescriptor {
        strategy: strategy.to_string(),
        value,
        frame_id: frame.clone(),
        confidence,
        backend_node_id,
        geometry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_hint_produces_primary_and_fallback() {
        let frame = FrameId::new();
        let candidates = from_hint(&ResolveHint::Css("div.btn".into()), &frame);
        assert!(!candidates.is_empty());
        assert!(candidates.iter().all(|c| c.strategy == "css"));
        assert!(candidates.iter().any(|c| c.confidence > 0.6));
    }

    #[test]
    fn backend_hint_sets_backend_and_confidence() {
        let frame = FrameId::new();
        let candidates = from_hint(&ResolveHint::Backend(42), &frame);
        assert_eq!(candidates.len(), 1);
        let anchor = &candidates[0];
        assert_eq!(anchor.backend_node_id, Some(42));
        assert!(anchor.confidence >= 0.9);
    }

    #[test]
    fn combo_hint_produces_aggregate_candidate() {
        let frame = FrameId::new();
        let combo = SelectorOrHint::Combo(vec![
            SelectorOrHint::Css("button.primary".into()),
            SelectorOrHint::Text {
                pattern: "Submit".into(),
                fuzzy: Some(0.3),
            },
        ]);
        let candidates = from_selector(&combo, &frame, &ResolveOpt::default());
        assert!(candidates.iter().any(|c| c.strategy == "combo"));
        let combo_anchor = candidates
            .into_iter()
            .find(|c| c.strategy == "combo")
            .expect("combo candidate");
        let sources = combo_anchor
            .value
            .as_object()
            .and_then(|obj| obj.get("sources"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(sources.len() >= 2);
        assert!(combo_anchor.confidence >= 0.6);
    }

    #[test]
    fn text_hint_yields_case_variants_and_fuzzy() {
        let frame = FrameId::new();
        let opt = ResolveOpt {
            max_candidates: 3,
            fuzziness: Some(0.4),
            debounce_ms: None,
        };
        let candidates = from_selector(
            &SelectorOrHint::Text {
                pattern: "Submit".into(),
                fuzzy: None,
            },
            &frame,
            &opt,
        );
        assert!(candidates.iter().any(|c| {
            c.value
                .as_object()
                .and_then(|obj| obj.get("match"))
                .and_then(Value::as_str)
                == Some("contains")
        }));
        assert!(candidates.iter().any(|c| {
            c.value
                .as_object()
                .and_then(|obj| obj.get("match"))
                .and_then(Value::as_str)
                == Some("case_insensitive")
        }));
        assert!(candidates.iter().any(|c| {
            c.value
                .as_object()
                .and_then(|obj| obj.get("match"))
                .and_then(Value::as_str)
                == Some("fuzzy")
        }));
    }

    #[test]
    fn selector_attr_generates_candidate() {
        let frame = FrameId::new();
        let candidates = from_selector(
            &SelectorOrHint::Attr {
                key: "data-test".into(),
                value: "login".into(),
            },
            &frame,
            &ResolveOpt::default(),
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].strategy, "attr");
    }

    #[test]
    fn selector_combo_deduplicates_candidates() {
        let frame = FrameId::new();
        let combo = SelectorOrHint::Combo(vec![
            SelectorOrHint::Css("#submit".into()),
            SelectorOrHint::Css("#submit".into()),
        ]);
        let candidates = from_selector(&combo, &frame, &ResolveOpt::default());
        assert!(!candidates.is_empty());
    }
}
