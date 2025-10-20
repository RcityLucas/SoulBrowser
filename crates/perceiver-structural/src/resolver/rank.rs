use std::cmp::Ordering;
use std::collections::HashSet;

use serde_json::Value;

use crate::model::{AnchorDescriptor, AnchorGeometry, ScoreBreakdown, ScoreComponent};
use crate::policy::ScoreWeights;

pub fn rank_candidates(
    candidates: Vec<AnchorDescriptor>,
    weights: &ScoreWeights,
) -> Vec<(AnchorDescriptor, ScoreBreakdown)> {
    let mut scored: Vec<(ScoreBreakdown, AnchorDescriptor)> = candidates
        .into_iter()
        .map(|mut anchor| {
            let breakdown = score_candidate(&anchor, weights);
            anchor.confidence = breakdown.total;
            (breakdown, anchor)
        })
        .collect();

    scored.sort_by(|a, b| match b.0.total.partial_cmp(&a.0.total) {
        Some(ordering) => ordering,
        None => Ordering::Equal,
    });

    scored
        .into_iter()
        .map(|(score, anchor)| (anchor, score))
        .collect()
}

pub fn score_candidate(anchor: &AnchorDescriptor, weights: &ScoreWeights) -> ScoreBreakdown {
    let mut score = ScoreBreakdown::default();
    push_component(&mut score, "confidence", 1.0, anchor.confidence);

    if anchor.backend_node_id.is_some() {
        push_component(&mut score, "backend", weights.backend, 1.0);
    }

    if let Some(geometry) = &anchor.geometry {
        let factor = geometry_factor(geometry);
        if factor > 0.0 {
            push_component(&mut score, "geometry", weights.geometry, factor);
        }
        if factor > 0.0 {
            push_component(&mut score, "visibility", weights.visibility, factor);
        }
    }

    if let Value::Object(fields) = &anchor.value {
        if fields.contains_key("role") || fields.contains_key("name") {
            push_component(&mut score, "accessibility", weights.accessibility, 1.0);
        }
        if let Some(selector) = fields.get("selector") {
            if selector.is_string() {
                push_component(&mut score, "text", weights.text, 1.0);
            }
        }
        if fields
            .get("match")
            .and_then(Value::as_str)
            .map(|m| matches!(m, "contains" | "case_insensitive"))
            .unwrap_or(false)
        {
            push_component(&mut score, "text-fuzzy", weights.text * 0.5, 1.0);
        }
        if anchor.strategy == "combo" {
            if let Some(Value::Array(sources)) = fields.get("sources") {
                let mut seen = HashSet::new();
                for entry in sources.iter().filter_map(|value| value.as_object()) {
                    if let Some(strategy) = entry.get("strategy").and_then(Value::as_str) {
                        if !seen.insert(strategy) {
                            continue;
                        }
                        match strategy {
                            "css" => push_component(&mut score, "combo-css", weights.text, 0.7),
                            "text" => push_component(&mut score, "combo-text", weights.text, 0.8),
                            "aria" | "ax" => push_component(
                                &mut score,
                                "combo-accessibility",
                                weights.accessibility,
                                0.8,
                            ),
                            "backend" => {
                                push_component(&mut score, "combo-backend", weights.backend, 0.6)
                            }
                            "geometry" => {
                                push_component(&mut score, "combo-geometry", weights.geometry, 0.6)
                            }
                            _ => {}
                        }
                    }
                }
                let combo_bonus = (sources.len().min(3) as f32) / 3.0;
                push_component(
                    &mut score,
                    "combo-bonus",
                    weights.visibility,
                    combo_bonus * 0.5,
                );
            }
        }
    }

    match anchor.strategy.as_str() {
        "aria" => push_component(&mut score, "strategy-aria", weights.accessibility, 1.0),
        "text" => push_component(&mut score, "strategy-text", weights.text, 1.0),
        "geometry" => push_component(&mut score, "strategy-geometry", weights.geometry, 0.5),
        "backend" => push_component(&mut score, "strategy-backend", weights.backend, 0.5),
        "combo" => push_component(&mut score, "strategy-combo", weights.visibility, 0.6),
        _ => (),
    }

    score.total = score.total.clamp(0.0, 1.5);
    score
}

fn push_component(score: &mut ScoreBreakdown, label: &str, weight: f32, factor: f32) {
    if weight == 0.0 || factor == 0.0 {
        return;
    }
    let contribution = weight * factor;
    score.components.push(ScoreComponent {
        label: label.into(),
        weight,
        contribution,
    });
    score.total += contribution;
}

fn geometry_factor(geometry: &AnchorGeometry) -> f32 {
    if geometry.width <= 0.0 || geometry.height <= 0.0 {
        return 0.0;
    }
    let area = geometry.width.max(0.0) * geometry.height.max(0.0);
    if area <= 0.0 {
        return 0.0;
    }
    (area / 10_000.0).clamp(0.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use soulbrowser_core_types::FrameId;

    fn weights() -> ScoreWeights {
        ScoreWeights::default()
    }

    fn make_anchor(
        strategy: &str,
        confidence: f32,
        backend: Option<u64>,
        geometry: Option<AnchorGeometry>,
    ) -> AnchorDescriptor {
        AnchorDescriptor {
            strategy: strategy.to_string(),
            value: json!({ "selector": "#submit" }),
            frame_id: FrameId::new(),
            confidence,
            backend_node_id: backend,
            geometry,
        }
    }

    #[test]
    fn backend_anchor_wins_ranking() {
        let anchors = vec![
            make_anchor("css", 0.7, None, None),
            make_anchor("backend", 0.4, Some(99), None),
        ];

        let ranked = rank_candidates(anchors, &weights());
        assert_eq!(ranked[0].0.backend_node_id, Some(99));
        assert!(ranked[0].1.total > ranked[1].1.total);
    }

    #[test]
    fn geometry_improves_score() {
        let anchor_without_geom = make_anchor("css", 0.5, None, None);
        let anchor_with_geom = make_anchor(
            "css",
            0.5,
            None,
            Some(AnchorGeometry {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 30.0,
            }),
        );

        let score_plain = score_candidate(&anchor_without_geom, &weights());
        let score_geom = score_candidate(&anchor_with_geom, &weights());
        assert!(score_geom.total > score_plain.total);

        // Ensure the ranking pipeline writes back the boosted score.
        let ranked = rank_candidates(vec![anchor_without_geom, anchor_with_geom], &weights());
        assert!(ranked[0].0.geometry.is_some());
        assert!(ranked[0].1.total >= ranked[1].1.total);
    }
}
