//! Reasoning helpers for anchor scoring.
//!
//! 当前实现为占位，后续阶段将生成完整的 ScoreBreakdown → 文本理由映射。

use std::cmp::Ordering;

use crate::model::{ScoreBreakdown, ScoreComponent};

pub fn summarize(score: &ScoreBreakdown) -> String {
    if score.components.is_empty() {
        return format!("score={:.2}", score.total);
    }
    let mut components = score.components.clone();
    components.sort_by(|a, b| {
        b.contribution
            .partial_cmp(&a.contribution)
            .unwrap_or(Ordering::Equal)
    });

    let top_reasons: Vec<String> = components.iter().take(3).map(describe_component).collect();

    if top_reasons.is_empty() {
        format!("score={:.2}", score.total)
    } else {
        format!("score={:.2}; {}", score.total, top_reasons.join(", "))
    }
}

pub fn from_confidence(confidence: f32) -> ScoreBreakdown {
    ScoreBreakdown {
        total: confidence,
        components: vec![ScoreComponent {
            label: "confidence".into(),
            weight: 1.0,
            contribution: confidence,
        }],
    }
}

fn describe_component(component: &ScoreComponent) -> String {
    let contribution = component.contribution;
    let gain = format_gain(contribution);
    match component.label.as_str() {
        "confidence" => format!("seed confidence {}", gain),
        "backend" | "strategy-backend" => format!("backend id match {}", gain),
        "geometry" | "strategy-geometry" => format!("screen area {}", gain),
        "visibility" => format!("visible geometry {}", gain),
        "accessibility" | "strategy-aria" => format!("accessibility hints {}", gain),
        "text" | "strategy-text" => format!("text match {}", gain),
        "text-fuzzy" => format!("fuzzy text {}", gain),
        other => format!("{} {}", other, gain),
    }
}

fn format_gain(value: f32) -> String {
    format!("+{:.2}", value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_orders_top_reasons() {
        let mut score = ScoreBreakdown::default();
        score.total = 0.9;
        score.components = vec![
            ScoreComponent {
                label: "backend".into(),
                weight: 0.25,
                contribution: 0.25,
            },
            ScoreComponent {
                label: "geometry".into(),
                weight: 0.1,
                contribution: 0.15,
            },
            ScoreComponent {
                label: "text".into(),
                weight: 0.05,
                contribution: 0.05,
            },
        ];

        let summary = summarize(&score);
        assert!(summary.contains("backend id match +0.25"));
        assert!(summary.contains("screen area +0.15"));
    }
}
