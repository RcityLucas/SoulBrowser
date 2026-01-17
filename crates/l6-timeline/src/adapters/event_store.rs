use crate::errors::TlError;
use crate::model::{ArtifactsRefs, EventEnvelope, ReplayBundle, ReplayEvent};
use crate::ports::EventStorePort;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::Value;
use soulbrowser_core_types::{ActionId, SessionId, TaskId};
use soulbrowser_event_store::model::{
    ArtifactRef, EventEnvelope as EsEnvelope, EventScope, Filter, ReadHandle,
    ReplayBundle as EsReplayBundle,
};
use soulbrowser_event_store::{EsError, EventStore as RuntimeEventStore};
use std::sync::Arc;

const DEFAULT_TAIL_LIMIT: usize = 4_096;
const HOT_WINDOW_HINT_MINUTES: i64 = 30;

pub struct EventStoreAdapter {
    inner: Arc<dyn RuntimeEventStore>,
    tail_limit: usize,
}

impl EventStoreAdapter {
    pub fn new(inner: Arc<dyn RuntimeEventStore>) -> Self {
        Self {
            inner,
            tail_limit: DEFAULT_TAIL_LIMIT,
        }
    }

    fn map_err(err: EsError) -> TlError {
        TlError::Internal(err.to_string())
    }

    fn convert(events: Vec<EsEnvelope>) -> Vec<EventEnvelope> {
        events
            .into_iter()
            .enumerate()
            .map(|(idx, env)| EventEnvelope {
                action_id: env.scope.action.map(|ActionId(id)| id).unwrap_or_default(),
                kind: env.kind,
                seq: idx as i64,
                ts_mono: env.ts_mono.min(i64::MAX as u128) as i64,
                payload: env.payload,
            })
            .collect()
    }

    async fn tail_with_filter(&self, filter: Filter) -> Result<Vec<EventEnvelope>, TlError> {
        let events = self
            .inner
            .tail(self.tail_limit, Some(filter))
            .await
            .map_err(Self::map_err)?;
        Ok(Self::convert(events))
    }
}

#[async_trait]
impl EventStorePort for EventStoreAdapter {
    async fn by_action(&self, action_id: &str) -> Result<Vec<EventEnvelope>, TlError> {
        let events = self
            .inner
            .by_action(action_id)
            .await
            .map_err(Self::map_err)?;
        Ok(Self::convert(events))
    }

    async fn by_flow_window(&self, flow_id: &str) -> Result<Vec<EventEnvelope>, TlError> {
        let scope = EventScope {
            session: Some(SessionId(flow_id.to_string())),
            ..Default::default()
        };
        self.tail_with_filter(Filter {
            scope: Some(scope),
            ..Default::default()
        })
        .await
    }

    async fn by_task_window(&self, task_id: &str) -> Result<Vec<EventEnvelope>, TlError> {
        let scope = EventScope {
            task: Some(TaskId(task_id.to_string())),
            ..Default::default()
        };
        self.tail_with_filter(Filter {
            scope: Some(scope),
            ..Default::default()
        })
        .await
    }

    async fn export_range(
        &self,
        since: chrono::DateTime<chrono::Utc>,
        until: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<EventEnvelope>, TlError> {
        let ReadHandle { events, .. } = self
            .inner
            .export_range(since, until)
            .await
            .map_err(Self::map_err)?;
        Ok(Self::convert(events))
    }

    async fn replay_minimal(&self, action_id: &str) -> Result<ReplayBundle, TlError> {
        let bundle = self
            .inner
            .replay_minimal(action_id)
            .await
            .map_err(Self::map_err)?;
        Ok(convert_replay_bundle(bundle))
    }

    async fn hot_window_hint(
        &self,
    ) -> Result<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>), TlError> {
        let now = Utc::now();
        Ok((now - Duration::minutes(HOT_WINDOW_HINT_MINUTES), now))
    }
}

fn convert_replay_bundle(bundle: EsReplayBundle) -> ReplayBundle {
    let action_id = bundle.action.map(|ActionId(id)| id).unwrap_or_default();
    let timeline = bundle
        .timeline
        .into_iter()
        .map(convert_replay_event)
        .collect();
    let evidence = convert_artifacts(bundle.evidence);
    ReplayBundle {
        action_id,
        timeline,
        evidence,
        summary: bundle.summary,
    }
}

fn convert_replay_event(value: Value) -> ReplayEvent {
    let delta_ms = value
        .get("delta_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or_default();
    let kind = value
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let digest = value.get("digest").cloned().unwrap_or(Value::Null);
    ReplayEvent {
        delta_ms,
        kind,
        digest,
    }
}

fn convert_artifacts(artifacts: Vec<ArtifactRef>) -> ArtifactsRefs {
    let mut pix = Vec::new();
    let mut structs = Vec::new();
    for artifact in artifacts {
        match artifact.kind.as_str() {
            "pix" | "pixel" => pix.push(artifact.id),
            "struct" | "structure" => structs.push(artifact.id),
            _ => {}
        }
    }
    ArtifactsRefs { pix, structs }
}
