use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum View {
    Records,
    Timeline,
    Replay,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum By {
    Action {
        action_id: String,
    },
    Flow {
        flow_id: String,
    },
    Task {
        task_id: String,
    },
    Range {
        since: DateTime<Utc>,
        until: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportReq {
    pub view: View,
    pub by: By,
    pub policy_overrides: Option<JsonValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExportStats {
    pub total_lines: usize,
    pub total_actions: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExportResult {
    pub path: Option<String>,
    pub lines: Option<Vec<String>>,
    pub stats: ExportStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryDigest {
    pub view: View,
    pub selector_kind: &'static str,
}

impl QueryDigest {
    pub fn from_req(req: &ExportReq) -> Self {
        let selector_kind = match &req.by {
            By::Action { .. } => "action",
            By::Flow { .. } => "flow",
            By::Task { .. } => "task",
            By::Range { .. } => "range",
        };
        Self {
            view: req.view.clone(),
            selector_kind,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EventEnvelope {
    pub action_id: String,
    pub kind: String,
    pub seq: i64,
    pub ts_mono: i64,
    pub payload: JsonValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActionNode {
    pub tool: String,
    pub primitive: String,
    pub wait_tier: Option<String>,
    pub precheck: Option<JsonValue>,
    pub ok: Option<bool>,
    pub error: Option<String>,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtifactsRefs {
    pub pix: Vec<String>,
    pub structs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvidenceNode {
    pub post_signals: JsonValue,
    pub artifacts: ArtifactsRefs,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GateDigest {
    pub pass: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskDigest {
    pub completed: bool,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InsightDigest {
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DecisionNode {
    pub gate: Option<GateDigest>,
    pub task_completed: Option<TaskDigest>,
    pub insight: Option<InsightDigest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TimelineFrame {
    pub action: ActionNode,
    pub evidence: Option<EvidenceNode>,
    pub decision: Option<DecisionNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReplayEvent {
    pub delta_ms: i64,
    pub kind: String,
    pub digest: JsonValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReplayBundle {
    pub action_id: String,
    pub timeline: Vec<ReplayEvent>,
    pub evidence: ArtifactsRefs,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeaderLine {
    pub export_version: &'static str,
    pub generated_at: DateTime<Utc>,
    pub policy_snapshot: JsonValue,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordLine {
    pub stage: &'static str,
    pub action_id: String,
    pub kind: String,
    pub ts_mono: i64,
    pub payload: JsonValue,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineLine {
    pub action_id: String,
    pub frame: TimelineFrame,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayLine {
    pub bundle: ReplayBundle,
}

#[derive(Debug, Clone, Serialize)]
pub struct FooterLine {
    pub total_actions: usize,
    pub total_lines: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JsonlLine {
    Header(HeaderLine),
    Record(RecordLine),
    Timeline(TimelineLine),
    Replay(ReplayLine),
    Footer(FooterLine),
}
