use crate::model::{EventEnvelope, ReplayBundle};

/// Crafts a minimal replay bundle by summarising core events.
pub fn build_minimal(events: &[EventEnvelope]) -> ReplayBundle {
    let timeline = events
        .iter()
        .map(|env| {
            serde_json::json!({
                "event_id": env.event_id,
                "kind": env.kind,
                "ts_wall": env.ts_wall,
            })
        })
        .collect();
    ReplayBundle {
        action: events.first().and_then(|ev| ev.scope.action.clone()),
        timeline,
        evidence: Vec::new(),
        summary: Some(format!("{} events", events.len())),
    }
}
