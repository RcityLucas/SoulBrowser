use chrono::{DateTime, Utc};

use crate::model::{EventEnvelope, Filter};

/// Utility helpers for read queries operating on in-memory slices.
pub fn tail(events: &[EventEnvelope], limit: usize, filter: Option<&Filter>) -> Vec<EventEnvelope> {
    let mut out = Vec::new();
    for env in events.iter().rev() {
        if matches_filter(env, filter) {
            out.push(env.clone());
        }
        if out.len() == limit {
            break;
        }
    }
    out.reverse();
    out
}

pub fn since(
    events: &[EventEnvelope],
    ts: DateTime<Utc>,
    limit: usize,
    filter: Option<&Filter>,
) -> Vec<EventEnvelope> {
    let mut out = Vec::new();
    for env in events {
        if env.ts_wall < ts {
            continue;
        }
        if matches_filter(env, filter) {
            out.push(env.clone());
        }
        if out.len() == limit {
            break;
        }
    }
    out
}

fn matches_filter(env: &EventEnvelope, filter: Option<&Filter>) -> bool {
    filter.map(|f| f.matches(env)).unwrap_or(true)
}
