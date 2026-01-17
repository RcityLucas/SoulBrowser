use std::collections::VecDeque;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tracing::warn;

use soulbase_types::tenant::TenantId;

use crate::storage::{BrowserSessionEntity, StorageManager};
use crate::task_status::{OverlayPayload, OverlaySource, TaskStreamEvent, TaskStreamObserver};

use super::live::SessionLiveEvent;
use super::types::{
    CreateSessionRequest, LiveFramePayload, LiveOverlayEntry, RouteSummary, SessionRecord,
    SessionShareContext, SessionSnapshot, SessionStatus,
};

const MAX_OVERLAY_HISTORY: usize = 40;

#[derive(Clone)]
pub struct SessionService {
    tenant_id: String,
    backend: Arc<dyn crate::storage::BrowserStorage>,
    live_origin: Option<String>,
    handles: DashMap<String, Arc<SessionHandle>>,
    task_sessions: DashMap<String, String>,
}

impl SessionService {
    pub fn new(tenant_id: String, storage: Arc<StorageManager>) -> Self {
        let live_origin = std::env::var("SOULBROWSER_LIVE_BASE_URL")
            .ok()
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty());
        Self {
            tenant_id,
            backend: storage.backend(),
            live_origin,
            handles: DashMap::new(),
            task_sessions: DashMap::new(),
        }
    }

    pub async fn hydrate(&self) {
        let Ok(existing) = self.backend.list_sessions().await else {
            warn!("session service failed to load persisted sessions");
            return;
        };
        for entity in existing {
            if entity.tenant.0 != self.tenant_id {
                continue;
            }
            if let Some(record) = self.record_from_entity(&entity) {
                let handle = Arc::new(SessionHandle::new(record));
                self.handles.insert(entity.id.clone(), handle);
            }
        }
    }

    pub async fn create_session(&self, request: CreateSessionRequest) -> Result<SessionRecord> {
        let id = uuid::Uuid::new_v4().to_string();
        self.create_session_with_id(id, request).await
    }

    pub async fn create_session_with_id(
        &self,
        session_id: String,
        request: CreateSessionRequest,
    ) -> Result<SessionRecord> {
        let now = Utc::now();
        let metadata = StoredSessionMetadata {
            profile_id: request.profile_id.clone(),
            profile_label: request.profile_label.clone(),
            share_token: request
                .shared
                .unwrap_or(false)
                .then(|| generate_share_token()),
            last_task_id: None,
            last_event_at: None,
        };
        let subject_id = request
            .profile_id
            .clone()
            .unwrap_or_else(|| "runtime".to_string());
        let entity = BrowserSessionEntity {
            id: session_id.clone(),
            tenant: TenantId(self.tenant_id.clone()),
            subject_id: subject_id.clone(),
            created_at: now.timestamp_millis(),
            updated_at: now.timestamp_millis(),
            state: SessionStatus::Initializing.as_storage_state().to_string(),
            metadata: serde_json::to_value(&metadata)?,
        };
        self.backend
            .store_session(entity)
            .await
            .context("persisting session")?;

        let mut record = SessionRecord {
            id: session_id.clone(),
            tenant_id: self.tenant_id.clone(),
            subject_id: Some(subject_id),
            profile_id: request.profile_id.clone(),
            profile_label: request.profile_label.clone(),
            status: SessionStatus::Initializing,
            created_at: now,
            updated_at: now,
            last_event_at: None,
            last_task_id: None,
            live_path: live_path_for(&session_id),
            share_token: metadata.share_token.clone(),
            share_url: metadata
                .share_token
                .as_ref()
                .and_then(|token| self.compose_share_url(&session_id, token)),
            share_path: metadata
                .share_token
                .as_ref()
                .map(|token| share_path_for(&session_id, token)),
        };
        let handle = Arc::new(SessionHandle::new(record.clone()));
        handle.emit_snapshot();
        self.handles.insert(session_id, handle);
        record.share_url = record
            .share_token
            .as_ref()
            .and_then(|token| self.compose_share_url(&record.id, token));
        Ok(record)
    }

    pub fn list(&self) -> Vec<SessionRecord> {
        self.handles
            .iter()
            .map(|entry| entry.value().record())
            .collect()
    }

    pub fn get(&self, session_id: &str) -> Option<SessionRecord> {
        self.handles.get(session_id).map(|handle| handle.record())
    }

    pub fn snapshot(&self, session_id: &str) -> Option<SessionSnapshot> {
        self.handles.get(session_id).map(|handle| handle.snapshot())
    }

    pub fn subscribe(&self, session_id: &str) -> Option<broadcast::Receiver<SessionLiveEvent>> {
        self.handles
            .get(session_id)
            .map(|handle| handle.sender.subscribe())
    }

    pub fn bind_task(&self, session_id: &str, task_id: &str) {
        if let Some(handle) = self.handles.get(session_id) {
            handle.bind_task(task_id);
        }
        self.task_sessions
            .insert(task_id.to_string(), session_id.to_string());
    }

    pub fn unbind_task(&self, task_id: &str) {
        self.task_sessions.remove(task_id);
    }

    pub async fn issue_share_link(&self, session_id: &str) -> Result<SessionShareContext> {
        let handle = self
            .handles
            .get(session_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow!("session not found"))?;
        let token = generate_share_token();
        {
            let mut record = handle.inner.record.write();
            record.share_token = Some(token.clone());
            record.share_path = Some(share_path_for(&record.id, &token));
            record.share_url = self.compose_share_url(&record.id, &token);
            record.updated_at = Utc::now();
        }
        let record = handle.record();
        self.persist_record(record).await?;
        Ok(handle.share_context(self.live_origin.as_ref()))
    }

    pub async fn revoke_share_link(&self, session_id: &str) -> Result<SessionShareContext> {
        let handle = self
            .handles
            .get(session_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow!("session not found"))?;
        {
            let mut record = handle.inner.record.write();
            record.share_token = None;
            record.share_path = None;
            record.share_url = None;
            record.updated_at = Utc::now();
        }
        let record = handle.record();
        self.persist_record(record).await?;
        Ok(handle.share_context(self.live_origin.as_ref()))
    }

    fn ensure_handle(&self, session_id: &str) -> Arc<SessionHandle> {
        if let Some(handle) = self.handles.get(session_id) {
            return handle.value().clone();
        }
        let now = Utc::now();
        let record = SessionRecord {
            id: session_id.to_string(),
            tenant_id: self.tenant_id.clone(),
            subject_id: None,
            profile_id: None,
            profile_label: None,
            status: SessionStatus::Initializing,
            created_at: now,
            updated_at: now,
            last_event_at: None,
            last_task_id: None,
            live_path: live_path_for(session_id),
            share_token: None,
            share_url: None,
            share_path: None,
        };
        let handle = Arc::new(SessionHandle::new(record));
        self.handles.insert(session_id.to_string(), handle.clone());
        handle
    }

    fn compose_share_url(&self, session_id: &str, token: &str) -> Option<String> {
        let origin = self.live_origin.as_ref()?;
        Some(format!(
            "{}/{}",
            origin.trim_end_matches('/'),
            share_path_for(session_id, token).trim_start_matches('/')
        ))
    }

    fn record_from_entity(&self, entity: &BrowserSessionEntity) -> Option<SessionRecord> {
        let metadata: StoredSessionMetadata = serde_json::from_value(entity.metadata.clone())
            .unwrap_or_else(|_| StoredSessionMetadata::default());
        Some(SessionRecord {
            id: entity.id.clone(),
            tenant_id: entity.tenant.0.clone(),
            subject_id: Some(entity.subject_id.clone()),
            profile_id: metadata.profile_id.clone(),
            profile_label: metadata.profile_label.clone(),
            status: SessionStatus::from_storage_state(&entity.state),
            created_at: timestamp_to_datetime(entity.created_at),
            updated_at: timestamp_to_datetime(entity.updated_at),
            last_event_at: metadata
                .last_event_at
                .as_deref()
                .and_then(parse_datetime_opt),
            last_task_id: metadata.last_task_id.clone(),
            live_path: live_path_for(&entity.id),
            share_token: metadata.share_token.clone(),
            share_url: metadata
                .share_token
                .as_deref()
                .and_then(|token| self.compose_share_url(&entity.id, token)),
            share_path: metadata
                .share_token
                .as_ref()
                .map(|token| share_path_for(&entity.id, token)),
        })
    }

    async fn persist_record(&self, record: SessionRecord) -> Result<()> {
        let metadata = StoredSessionMetadata {
            profile_id: record.profile_id.clone(),
            profile_label: record.profile_label.clone(),
            share_token: record.share_token.clone(),
            last_task_id: record.last_task_id.clone(),
            last_event_at: record.last_event_at.map(|ts| ts.to_rfc3339()),
        };
        let entity = BrowserSessionEntity {
            id: record.id.clone(),
            tenant: TenantId(self.tenant_id.clone()),
            subject_id: record
                .subject_id
                .clone()
                .unwrap_or_else(|| "runtime".to_string()),
            created_at: record.created_at.timestamp_millis(),
            updated_at: record.updated_at.timestamp_millis(),
            state: record.status.as_storage_state().to_string(),
            metadata: json!(metadata),
        };
        self.backend
            .update_session(entity)
            .await
            .context("update session")
    }

    fn handle_observation(
        &self,
        task_id: &str,
        observation: &crate::task_status::ObservationPayload,
    ) {
        if observation
            .content_type
            .as_deref()
            .map(|ct| !ct.starts_with("image/"))
            .unwrap_or(true)
        {
            return;
        }
        let base64 = observation
            .artifact
            .get("data_base64")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let Some(data_base64) = base64 else {
            return;
        };
        let route = observation.artifact.get("route");
        let Some(route_value) = route else {
            return;
        };
        let Some(session_id) = route_value.get("session").and_then(|value| value.as_str()) else {
            return;
        };
        let page = route_value
            .get("page")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let frame = route_value
            .get("frame")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let handle = self.ensure_handle(session_id);
        let overlays = handle.overlay_snapshot();
        let payload = LiveFramePayload {
            session_id: session_id.to_string(),
            task_id: Some(task_id.to_string()),
            recorded_at: observation.recorded_at,
            screenshot_base64: data_base64,
            route: Some(RouteSummary {
                session: session_id.to_string(),
                page,
                frame,
            }),
            overlays,
        };
        handle.push_frame(payload);
        handle.touch_event(task_id, observation.recorded_at);
    }

    fn handle_overlay(&self, overlay: &OverlayPayload) {
        let route = overlay.data.get("route");
        let Some(route_value) = route else {
            return;
        };
        let Some(session_id) = route_value.get("session").and_then(|value| value.as_str()) else {
            return;
        };
        let handle = self.ensure_handle(session_id);
        let entry = LiveOverlayEntry {
            session_id: session_id.to_string(),
            recorded_at: overlay.recorded_at,
            task_id: Some(overlay.task_id.clone()),
            source: overlay_source_label(overlay.source),
            data: overlay.data.clone(),
        };
        handle.push_overlay(entry);
    }

    fn handle_message_state(&self, task_id: &str, state: &Value) {
        let Some(session_id) = self
            .task_sessions
            .get(task_id)
            .map(|entry| entry.value().clone())
            .or_else(|| self.find_session_by_task(task_id))
        else {
            return;
        };
        let handle = self.ensure_handle(&session_id);
        handle.update_message_state(state.clone());
    }

    fn find_session_by_task(&self, task_id: &str) -> Option<String> {
        for entry in self.handles.iter() {
            if entry.value().record().last_task_id.as_deref() == Some(task_id) {
                return Some(entry.key().clone());
            }
        }
        None
    }
}

impl TaskStreamObserver for SessionService {
    fn handle(&self, task_id: &str, event: &TaskStreamEvent) {
        match event {
            TaskStreamEvent::Observation { observation } => {
                self.handle_observation(task_id, observation);
            }
            TaskStreamEvent::Overlay { overlay } => {
                self.handle_overlay(overlay);
            }
            TaskStreamEvent::MessageState { state } => {
                self.handle_message_state(task_id, state);
            }
            _ => {}
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct StoredSessionMetadata {
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    profile_label: Option<String>,
    #[serde(default)]
    share_token: Option<String>,
    #[serde(default)]
    last_task_id: Option<String>,
    #[serde(default)]
    last_event_at: Option<String>,
}

struct SessionHandle {
    inner: SessionHandleInner,
    sender: broadcast::Sender<SessionLiveEvent>,
}

struct SessionHandleInner {
    record: RwLock<SessionRecord>,
    overlays: Mutex<VecDeque<LiveOverlayEntry>>,
    last_frame: Mutex<Option<LiveFramePayload>>,
    message_state: Mutex<Option<serde_json::Value>>,
}

impl SessionHandle {
    fn new(record: SessionRecord) -> Self {
        let (sender, _) = broadcast::channel(64);
        Self {
            inner: SessionHandleInner {
                record: RwLock::new(record),
                overlays: Mutex::new(VecDeque::new()),
                last_frame: Mutex::new(None),
                message_state: Mutex::new(None),
            },
            sender,
        }
    }

    fn record(&self) -> SessionRecord {
        self.inner.record.read().clone()
    }

    fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            session: self.inner.record.read().clone(),
            overlays: self.overlay_snapshot(),
            last_frame: self.inner.last_frame.lock().clone(),
            message_state: self.inner.message_state.lock().clone(),
        }
    }

    fn overlay_snapshot(&self) -> Vec<LiveOverlayEntry> {
        self.inner.overlays.lock().iter().cloned().collect()
    }

    fn push_overlay(&self, overlay: LiveOverlayEntry) {
        {
            let mut guard = self.inner.overlays.lock();
            guard.push_back(overlay.clone());
            while guard.len() > MAX_OVERLAY_HISTORY {
                guard.pop_front();
            }
        }
        let _ = self.sender.send(SessionLiveEvent::Overlay { overlay });
    }

    fn push_frame(&self, frame: LiveFramePayload) {
        {
            let mut guard = self.inner.last_frame.lock();
            *guard = Some(frame.clone());
        }
        let _ = self.sender.send(SessionLiveEvent::Frame { frame });
    }

    fn update_message_state(&self, state: serde_json::Value) {
        {
            let mut guard = self.inner.message_state.lock();
            *guard = Some(state.clone());
        }
        let session_id = self.inner.record.read().id.clone();
        let _ = self
            .sender
            .send(SessionLiveEvent::MessageState { session_id, state });
    }

    fn touch_event(&self, task_id: &str, at: DateTime<Utc>) {
        self.set_status(SessionStatus::Active);
        {
            let mut record = self.inner.record.write();
            record.last_event_at = Some(at);
            record.last_task_id = Some(task_id.to_string());
            record.updated_at = at;
        }
        self.emit_status();
    }

    fn bind_task(&self, task_id: &str) {
        let mut record = self.inner.record.write();
        record.last_task_id = Some(task_id.to_string());
        record.updated_at = Utc::now();
    }

    fn set_status(&self, status: SessionStatus) {
        let mut record = self.inner.record.write();
        if record.status == status {
            return;
        }
        record.status = status;
        record.updated_at = Utc::now();
        drop(record);
        self.emit_status();
    }

    fn emit_status(&self) {
        let record = self.inner.record.read();
        let _ = self.sender.send(SessionLiveEvent::Status {
            session_id: record.id.clone(),
            status: record.status,
        });
    }

    fn emit_snapshot(&self) {
        let snapshot = self.snapshot();
        let _ = self.sender.send(SessionLiveEvent::Snapshot { snapshot });
    }

    fn share_context(&self, origin: Option<&String>) -> SessionShareContext {
        let record = self.inner.record.read();
        SessionShareContext {
            session_id: record.id.clone(),
            live_path: record.live_path.clone(),
            share_token: record.share_token.clone(),
            share_url: record
                .share_token
                .as_ref()
                .and_then(|token| origin.map(|o| compose_absolute(o, &record.id, token))),
        }
    }
}

fn live_path_for(session_id: &str) -> String {
    format!("/api/sessions/{}/live", session_id)
}

fn share_path_for(session_id: &str, token: &str) -> String {
    format!("/api/sessions/{}/live?share={}", session_id, token)
}

fn compose_absolute(origin: &str, session_id: &str, token: &str) -> String {
    format!(
        "{}/{}",
        origin.trim_end_matches('/'),
        share_path_for(session_id, token).trim_start_matches('/')
    )
}

fn timestamp_to_datetime(millis: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(millis)
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap())
}

fn parse_datetime_opt(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}

fn generate_share_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn overlay_source_label(source: OverlaySource) -> String {
    match source {
        OverlaySource::Plan => "plan".to_string(),
        OverlaySource::Execution => "execution".to_string(),
    }
}

impl SessionStatus {
    fn as_storage_state(self) -> &'static str {
        match self {
            SessionStatus::Initializing => "initializing",
            SessionStatus::Active => "active",
            SessionStatus::Idle => "idle",
            SessionStatus::Completed => "completed",
            SessionStatus::Failed => "failed",
        }
    }

    fn from_storage_state(raw: &str) -> SessionStatus {
        match raw.to_ascii_lowercase().as_str() {
            "active" => SessionStatus::Active,
            "idle" => SessionStatus::Idle,
            "completed" => SessionStatus::Completed,
            "failed" => SessionStatus::Failed,
            _ => SessionStatus::Initializing,
        }
    }
}
