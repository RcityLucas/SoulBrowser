use crate::model::EventEnvelope;

pub fn apply_fold(events: Vec<EventEnvelope>, _fold_noise: bool) -> Vec<EventEnvelope> {
    // Placeholder for future noise folding. Currently returns the original sequence.
    events
}
