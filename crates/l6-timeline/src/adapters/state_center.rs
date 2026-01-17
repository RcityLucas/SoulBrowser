use crate::errors::TlError;
use crate::model::EventEnvelope;
use crate::ports::StateCenterPort;
use async_trait::async_trait;
use serde_json::json;
use soulbrowser_state_center::{
    DispatchEvent, DispatchStatus, PerceiverEvent, PerceiverEventKind, RegistryEvent, StateEvent,
};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct StateCenterAdapter {
    inner: Arc<soulbrowser_state_center::InMemoryStateCenter>,
}

impl StateCenterAdapter {
    pub fn new(inner: Arc<soulbrowser_state_center::InMemoryStateCenter>) -> Self {
        Self { inner }
    }

    fn convert_events(events: Vec<StateEvent>) -> Vec<EventEnvelope> {
        events
            .into_iter()
            .enumerate()
            .map(|(idx, event)| match event {
                StateEvent::Dispatch(dispatch) => Self::dispatch_to_envelope(idx as i64, dispatch),
                StateEvent::Registry(registry) => Self::registry_to_envelope(idx as i64, registry),
                StateEvent::Perceiver(perceiver) => {
                    Self::perceiver_to_envelope(idx as i64, perceiver)
                }
            })
            .collect()
    }

    fn dispatch_to_envelope(seq: i64, dispatch: DispatchEvent) -> EventEnvelope {
        let ts_mono = ts_mono(dispatch.recorded_at);
        let DispatchEvent {
            action_id,
            task_id,
            status,
            route,
            tool,
            mutex_key,
            attempts,
            wait_ms,
            run_ms,
            pending,
            slots_available,
            error,
            output,
            recorded_at: _,
        } = dispatch;
        let mut payload = json!({
            "tool": tool,
            "mutex_key": mutex_key,
            "attempts": attempts,
            "wait_ms": wait_ms,
            "run_ms": run_ms,
            "pending": pending,
            "slots_available": slots_available,
        });
        if let Some(error) = error {
            payload["error"] = json!(error.to_string());
        }
        if let Some(task) = task_id {
            payload["task_id"] = json!(task);
        }
        if let Some(output) = output {
            payload["output"] = output;
        }
        let kind = match status {
            DispatchStatus::Success => "SC_DISPATCH_SUCCESS",
            DispatchStatus::Failure => "SC_DISPATCH_FAILURE",
        };
        payload["route"] = json!({
            "session": route.session.0,
            "page": route.page.0,
            "frame": route.frame.0,
            "mutex": route.mutex_key,
        });
        EventEnvelope {
            action_id: action_id.0,
            kind: kind.into(),
            seq,
            ts_mono,
            payload,
        }
    }

    fn registry_to_envelope(seq: i64, registry: RegistryEvent) -> EventEnvelope {
        let ts_mono = ts_mono(registry.recorded_at);
        let payload = json!({
            "action": format!("{:?}", registry.action),
            "session": registry.session.as_ref().map(|id| id.0.clone()),
            "page": registry.page.as_ref().map(|id| id.0.clone()),
            "frame": registry.frame.as_ref().map(|id| id.0.clone()),
            "note": registry.note,
        });
        EventEnvelope {
            action_id: String::new(),
            kind: "SC_REGISTRY".into(),
            seq,
            ts_mono,
            payload,
        }
    }

    fn perceiver_to_envelope(seq: i64, perceiver: PerceiverEvent) -> EventEnvelope {
        let ts_mono = ts_mono(perceiver.recorded_at);
        let (kind, payload) = match perceiver.kind {
            PerceiverEventKind::Resolve {
                strategy,
                score,
                candidate_count,
                cache_hit,
                breakdown,
                reason,
            } => (
                "SC_PERCEIVER_RESOLVE",
                json!({
                    "strategy": strategy,
                    "score": score,
                    "candidate_count": candidate_count,
                    "cache_hit": cache_hit,
                    "breakdown": breakdown,
                    "reason": reason,
                }),
            ),
            PerceiverEventKind::Judge {
                check,
                ok,
                reason,
                facts,
            } => (
                "SC_PERCEIVER_JUDGE",
                json!({
                    "check": check,
                    "ok": ok,
                    "reason": reason,
                    "facts": facts,
                }),
            ),
            PerceiverEventKind::Snapshot { cache_hit } => {
                ("SC_PERCEIVER_SNAPSHOT", json!({ "cache_hit": cache_hit }))
            }
            PerceiverEventKind::Diff {
                change_count,
                changes,
            } => (
                "SC_PERCEIVER_DIFF",
                json!({
                    "change_count": change_count,
                    "changes": changes,
                }),
            ),
        };
        EventEnvelope {
            action_id: String::new(),
            kind: kind.into(),
            seq,
            ts_mono,
            payload,
        }
    }
}

fn ts_mono(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|dur| dur.as_millis())
        .unwrap_or_default()
        .min(i64::MAX as u128) as i64
}

#[async_trait]
impl StateCenterPort for StateCenterAdapter {
    async fn tail(&self, limit: usize) -> Result<Vec<EventEnvelope>, TlError> {
        let mut snapshot = self.inner.snapshot();
        if snapshot.len() > limit {
            snapshot = snapshot.split_off(snapshot.len() - limit);
        }
        Ok(Self::convert_events(snapshot))
    }
}
