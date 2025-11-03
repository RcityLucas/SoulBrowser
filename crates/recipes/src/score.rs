use crate::model::{OutcomeReason, Scores};

const PASS_QUALITY: f32 = 0.9;
const PASS_SAFETY: f32 = 0.95;
const FAIL_QUALITY_DECAY: f32 = 0.7;
const FAIL_SAFETY_DECAY: f32 = 0.8;
const FAIL_FRESHNESS_DECAY: f32 = 0.6;

pub fn bootstrap(outcome: &OutcomeReason) -> Scores {
    match outcome {
        OutcomeReason::Pass => Scores {
            quality: PASS_QUALITY,
            safety: PASS_SAFETY,
            freshness: 1.0,
            support_n: 1,
        },
        OutcomeReason::Fail { .. } => Scores {
            quality: 0.2,
            safety: 0.4,
            freshness: 0.5,
            support_n: 0,
        },
    }
}

pub fn apply_outcome(prev: &Scores, outcome: &OutcomeReason, _freshness_tau_sec: u64) -> Scores {
    match outcome {
        OutcomeReason::Pass => {
            let new_support = prev.support_n.saturating_add(1);
            let support_f = new_support.max(1) as f32;
            let quality = ((prev.quality * prev.support_n as f32) + PASS_QUALITY) / support_f;
            let safety = ((prev.safety * prev.support_n as f32) + PASS_SAFETY) / support_f;
            Scores {
                quality: clamp01(quality),
                safety: clamp01(safety),
                freshness: 1.0,
                support_n: new_support,
            }
        }
        OutcomeReason::Fail { .. } => Scores {
            quality: clamp01(prev.quality * FAIL_QUALITY_DECAY),
            safety: clamp01(prev.safety * FAIL_SAFETY_DECAY),
            freshness: clamp01(prev.freshness * FAIL_FRESHNESS_DECAY),
            support_n: prev.support_n,
        },
    }
}

fn clamp01(value: f32) -> f32 {
    value.max(0.0).min(1.0)
}
