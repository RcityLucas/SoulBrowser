use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use soulbrowser_core_types::{ActionId, FrameId, PageId, SessionId, TaskId};

/// Severity level for stored events.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Info
    }
}

impl LogLevel {
    pub fn priority(self) -> u8 {
        match self {
            LogLevel::Trace => 0,
            LogLevel::Debug => 1,
            LogLevel::Info => 2,
            LogLevel::Warn => 3,
            LogLevel::Error => 4,
        }
    }
}

/// Origin of an event inside the SoulBrowser stack.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    L0,
    L1,
    L2,
    L3,
    L4,
    L5,
    #[serde(other)]
    Other,
}

/// Scope information for an observation/event.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EventScope {
    pub session: Option<SessionId>,
    pub page: Option<PageId>,
    pub frame: Option<FrameId>,
    pub task: Option<TaskId>,
    pub action: Option<ActionId>,
}

/// Lightweight reference to a persisted artifact.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub id: String,
    pub kind: String,
    pub hash: Option<String>,
    pub hint: Option<String>,
}

/// Unified envelope for events injected into the event store.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: String,
    pub ts_mono: u128,
    pub ts_wall: DateTime<Utc>,
    pub scope: EventScope,
    pub source: EventSource,
    pub kind: String,
    pub level: LogLevel,
    pub payload: serde_json::Value,
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub tags: Vec<(String, String)>,
}

impl EventEnvelope {
    /// Estimates an idempotency key for envelopes lacking explicit identifiers.
    pub fn idempotency_hint(&self) -> String {
        format!("{}:{}", self.event_id, self.ts_mono)
    }
}

/// Observation payload compatible with L1 contracts.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Observation {
    pub ok: bool,
    pub tool: String,
    #[serde(default)]
    pub signals: serde_json::Value,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub meta: serde_json::Value,
    pub error: Option<serde_json::Value>,
}

/// Metadata describing the append call.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppendMeta {
    pub source_mod: Option<String>,
    pub hot_only: bool,
    pub idempotency_key: Option<String>,
}

/// Acknowledgement returned to writers.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppendAck {
    pub event_id: String,
    pub accepted: bool,
    pub dropped_reason: Option<String>,
}

/// Acknowledgement for batch operations.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BatchAck {
    pub accepted: usize,
    pub dropped: usize,
    #[serde(default)]
    pub errors: SmallVec<[String; 4]>,
}

/// Filter constraints accepted by read queries.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Filter {
    pub kinds: Option<Vec<String>>,
    pub scope: Option<EventScope>,
    pub level_ge: Option<LogLevel>,
}

impl Filter {
    pub fn matches(&self, env: &EventEnvelope) -> bool {
        if let Some(kinds) = &self.kinds {
            if !kinds.iter().any(|kind| kind == &env.kind) {
                return false;
            }
        }
        if let Some(level) = self.level_ge {
            if env.level.priority() < level.priority() {
                return false;
            }
        }
        if let Some(scope) = &self.scope {
            if let Some(expect) = &scope.session {
                if env.scope.session.as_ref() != Some(expect) {
                    return false;
                }
            }
            if let Some(expect) = &scope.page {
                if env.scope.page.as_ref() != Some(expect) {
                    return false;
                }
            }
            if let Some(expect) = &scope.frame {
                if env.scope.frame.as_ref() != Some(expect) {
                    return false;
                }
            }
            if let Some(expect) = &scope.task {
                if env.scope.task.as_ref() != Some(expect) {
                    return false;
                }
            }
            if let Some(expect) = &scope.action {
                if env.scope.action.as_ref() != Some(expect) {
                    return false;
                }
            }
        }
        true
    }
}

/// Handle returned for range exports (placeholder).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadHandle {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    #[serde(default)]
    pub events: Vec<EventEnvelope>,
}

/// Batch returned by streaming range exports.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamBatch {
    #[serde(default)]
    pub events: Vec<EventEnvelope>,
    pub is_last: bool,
}

/// Bundle returned by minimal replay queries.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReplayBundle {
    pub action: Option<ActionId>,
    #[serde(default)]
    pub timeline: Vec<serde_json::Value>,
    #[serde(default)]
    pub evidence: Vec<ArtifactRef>,
    pub summary: Option<String>,
}
