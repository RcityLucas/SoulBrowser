use crate::config::DropPolicy;
use crate::model::EventEnvelope;

/// Evaluates whether an event should be dropped given the current utilization.
pub fn should_drop(envelope: &EventEnvelope, utilization: f32, policy: &DropPolicy) -> bool {
    if utilization < policy.hot_high_watermark {
        return false;
    }
    if policy.should_protect(&envelope.kind) {
        return false;
    }
    policy
        .low_priority_kinds
        .iter()
        .any(|kind| kind == &envelope.kind)
}
