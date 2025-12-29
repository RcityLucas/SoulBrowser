use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Initializing,
    Active,
    Idle,
    Completed,
    Failed,
}

impl Default for SessionStatus {
    fn default() -> Self {
        SessionStatus::Initializing
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub tenant_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_label: Option<String>,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_task_id: Option<String>,
    pub live_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionSnapshot {
    pub session: SessionRecord,
    #[serde(default)]
    pub overlays: Vec<LiveOverlayEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_frame: Option<LiveFramePayload>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub profile_label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub shared: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionShareContext {
    pub session_id: String,
    pub live_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LiveFramePayload {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub recorded_at: DateTime<Utc>,
    pub screenshot_base64: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<RouteSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<LiveOverlayEntry>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteSummary {
    pub session: String,
    pub page: Option<String>,
    pub frame: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LiveOverlayEntry {
    pub session_id: String,
    pub recorded_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub source: String,
    pub data: Value,
}
