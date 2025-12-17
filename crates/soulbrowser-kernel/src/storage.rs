//! Storage and persistence module
#![allow(dead_code)]
//!
//! Manages data persistence using soulbase-storage

use crate::errors::SoulBrowserError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use soulbase_storage::model::Entity;
use soulbase_types::tenant::TenantId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Query parameters for event queries
#[derive(Debug, Clone)]
pub struct QueryParams {
    pub session_id: Option<String>,
    pub event_type: Option<String>,
    pub from_timestamp: Option<i64>,
    pub to_timestamp: Option<i64>,
    pub limit: usize,
    pub offset: usize,
}

impl Default for QueryParams {
    fn default() -> Self {
        Self {
            session_id: None,
            event_type: None,
            from_timestamp: None,
            to_timestamp: None,
            limit: 0,
            offset: 0,
        }
    }
}

fn matches_query(event: &BrowserEvent, params: &QueryParams) -> bool {
    if let Some(session_id) = &params.session_id {
        if &event.session_id != session_id {
            return false;
        }
    }

    if let Some(event_type) = &params.event_type {
        if &event.event_type != event_type {
            return false;
        }
    }

    if let Some(from) = params.from_timestamp {
        if event.timestamp < from {
            return false;
        }
    }

    if let Some(to) = params.to_timestamp {
        if event.timestamp > to {
            return false;
        }
    }

    true
}

/// Browser event entity for soulbase-storage
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserEvent {
    pub id: String,
    pub tenant: TenantId,
    pub session_id: String,
    pub timestamp: i64,
    pub event_type: String,
    pub data: serde_json::Value,
    pub sequence: u64,
    pub tags: Vec<String>,
}

impl Entity for BrowserEvent {
    const TABLE: &'static str = "browser_events";

    fn id(&self) -> &str {
        &self.id
    }

    fn tenant(&self) -> &TenantId {
        &self.tenant
    }
}

/// Browser session entity for soulbase-storage
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserSessionEntity {
    pub id: String,
    pub tenant: TenantId,
    pub subject_id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub state: String,
    pub metadata: serde_json::Value,
}

impl Entity for BrowserSessionEntity {
    const TABLE: &'static str = "browser_sessions";

    fn id(&self) -> &str {
        &self.id
    }

    fn tenant(&self) -> &TenantId {
        &self.tenant
    }
}

/// Storage backend trait for browser data
#[async_trait]
pub trait BrowserStorage: Send + Sync {
    /// Store an event
    async fn store_event(&self, event: BrowserEvent) -> Result<(), SoulBrowserError>;

    /// Get an event by ID
    async fn get_event(&self, id: &str) -> Result<Option<BrowserEvent>, SoulBrowserError>;

    /// Query events
    async fn query_events(
        &self,
        params: QueryParams,
    ) -> Result<Vec<BrowserEvent>, SoulBrowserError>;

    /// Store a session
    async fn store_session(&self, session: BrowserSessionEntity) -> Result<(), SoulBrowserError>;

    /// Get a session by ID
    async fn get_session(&self, id: &str)
        -> Result<Option<BrowserSessionEntity>, SoulBrowserError>;

    /// Update session
    async fn update_session(&self, session: BrowserSessionEntity) -> Result<(), SoulBrowserError>;

    /// List all sessions
    async fn list_sessions(&self) -> Result<Vec<BrowserSessionEntity>, SoulBrowserError>;
}

/// In-memory storage implementation
pub struct InMemoryStorage {
    events: Arc<RwLock<HashMap<String, BrowserEvent>>>,
    sessions: Arc<RwLock<HashMap<String, BrowserSessionEntity>>>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {
            events: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl BrowserStorage for InMemoryStorage {
    async fn store_event(&self, event: BrowserEvent) -> Result<(), SoulBrowserError> {
        let mut events = self.events.write().await;
        events.insert(event.id.clone(), event);
        Ok(())
    }

    async fn get_event(&self, id: &str) -> Result<Option<BrowserEvent>, SoulBrowserError> {
        let events = self.events.read().await;
        Ok(events.get(id).cloned())
    }

    async fn query_events(
        &self,
        params: QueryParams,
    ) -> Result<Vec<BrowserEvent>, SoulBrowserError> {
        let events = self.events.read().await;

        let mut filtered: Vec<BrowserEvent> = events
            .values()
            .cloned()
            .filter(|event| matches_query(event, &params))
            .collect();

        filtered.sort_by(|a, b| {
            a.timestamp
                .cmp(&b.timestamp)
                .then_with(|| a.sequence.cmp(&b.sequence))
        });

        let offset = params.offset.min(filtered.len());
        let remaining = filtered.len().saturating_sub(offset);
        let limit = if params.limit == 0 {
            remaining
        } else {
            params.limit.min(remaining)
        };

        Ok(filtered.into_iter().skip(offset).take(limit).collect())
    }

    async fn store_session(&self, session: BrowserSessionEntity) -> Result<(), SoulBrowserError> {
        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id.clone(), session);
        Ok(())
    }

    async fn get_session(
        &self,
        id: &str,
    ) -> Result<Option<BrowserSessionEntity>, SoulBrowserError> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(id).cloned())
    }

    async fn update_session(&self, session: BrowserSessionEntity) -> Result<(), SoulBrowserError> {
        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id.clone(), session);
        Ok(())
    }

    async fn list_sessions(&self) -> Result<Vec<BrowserSessionEntity>, SoulBrowserError> {
        let sessions = self.sessions.read().await;
        Ok(sessions.values().cloned().collect())
    }
}

/// File-based storage implementation
pub struct FileStorage {
    base_path: PathBuf,
}

impl FileStorage {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    fn events_dir(&self) -> PathBuf {
        self.base_path.join("events")
    }

    fn sessions_dir(&self) -> PathBuf {
        self.base_path.join("sessions")
    }
}

#[async_trait]
impl BrowserStorage for FileStorage {
    async fn store_event(&self, event: BrowserEvent) -> Result<(), SoulBrowserError> {
        let dir = self.events_dir();
        tokio::fs::create_dir_all(&dir).await?;

        let path = dir.join(format!("{}.json", event.id));
        let json = serde_json::to_string_pretty(&event)?;
        tokio::fs::write(path, json).await?;

        Ok(())
    }

    async fn get_event(&self, id: &str) -> Result<Option<BrowserEvent>, SoulBrowserError> {
        let path = self.events_dir().join(format!("{}.json", id));

        if !path.exists() {
            return Ok(None);
        }

        let json = tokio::fs::read_to_string(path).await?;
        let event: BrowserEvent = serde_json::from_str(&json)?;
        Ok(Some(event))
    }

    async fn query_events(
        &self,
        params: QueryParams,
    ) -> Result<Vec<BrowserEvent>, SoulBrowserError> {
        let dir = self.events_dir();

        if !dir.exists() {
            return Ok(vec![]);
        }

        let mut events = Vec::new();
        let mut entries = tokio::fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            if entry.path().extension() == Some(std::ffi::OsStr::new("json")) {
                let json = tokio::fs::read_to_string(entry.path()).await?;
                if let Ok(event) = serde_json::from_str::<BrowserEvent>(&json) {
                    if matches_query(&event, &params) {
                        events.push(event);
                    }
                }
            }
        }

        events.sort_by(|a, b| {
            a.timestamp
                .cmp(&b.timestamp)
                .then_with(|| a.sequence.cmp(&b.sequence))
        });

        let offset = params.offset.min(events.len());
        let remaining = events.len().saturating_sub(offset);
        let limit = if params.limit == 0 {
            remaining
        } else {
            params.limit.min(remaining)
        };

        Ok(events.into_iter().skip(offset).take(limit).collect())
    }

    async fn store_session(&self, session: BrowserSessionEntity) -> Result<(), SoulBrowserError> {
        let dir = self.sessions_dir();
        tokio::fs::create_dir_all(&dir).await?;

        let path = dir.join(format!("{}.json", session.id));
        let json = serde_json::to_string_pretty(&session)?;
        tokio::fs::write(path, json).await?;

        Ok(())
    }

    async fn get_session(
        &self,
        id: &str,
    ) -> Result<Option<BrowserSessionEntity>, SoulBrowserError> {
        let path = self.sessions_dir().join(format!("{}.json", id));

        if !path.exists() {
            return Ok(None);
        }

        let json = tokio::fs::read_to_string(path).await?;
        let session: BrowserSessionEntity = serde_json::from_str(&json)?;
        Ok(Some(session))
    }

    async fn update_session(&self, session: BrowserSessionEntity) -> Result<(), SoulBrowserError> {
        self.store_session(session).await
    }

    async fn list_sessions(&self) -> Result<Vec<BrowserSessionEntity>, SoulBrowserError> {
        let dir = self.sessions_dir();

        if !dir.exists() {
            return Ok(vec![]);
        }

        let mut sessions = Vec::new();
        let mut entries = tokio::fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            if entry.path().extension() == Some(std::ffi::OsStr::new("json")) {
                let json = tokio::fs::read_to_string(entry.path()).await?;
                if let Ok(session) = serde_json::from_str::<BrowserSessionEntity>(&json) {
                    sessions.push(session);
                }
            }
        }

        Ok(sessions)
    }
}

/// Storage manager that can switch between backends
pub struct StorageManager {
    backend: Arc<dyn BrowserStorage>,
}

impl StorageManager {
    /// Create with in-memory backend
    pub fn in_memory() -> Self {
        Self {
            backend: Arc::new(InMemoryStorage::new()),
        }
    }

    /// Create with file backend
    pub fn file_based(path: PathBuf) -> Self {
        Self {
            backend: Arc::new(FileStorage::new(path)),
        }
    }

    /// Get the storage backend
    pub fn backend(&self) -> Arc<dyn BrowserStorage> {
        self.backend.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_storage() {
        let storage = InMemoryStorage::new();

        let event = BrowserEvent {
            id: "test-1".to_string(),
            tenant: TenantId("tenant-1".to_string()),
            session_id: "session-1".to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            event_type: "click".to_string(),
            data: serde_json::json!({"target": "#button"}),
            sequence: 1,
            tags: vec!["test".to_string()],
        };

        // Store event
        storage.store_event(event.clone()).await.unwrap();

        // Get event
        let retrieved = storage.get_event("test-1").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "test-1");

        // Query events
        let params = QueryParams::default();
        let events = storage.query_events(params).await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_session_storage() {
        let storage = InMemoryStorage::new();

        let session = BrowserSessionEntity {
            id: "session-1".to_string(),
            tenant: TenantId("tenant-1".to_string()),
            subject_id: "user-1".to_string(),
            created_at: chrono::Utc::now().timestamp_millis(),
            updated_at: chrono::Utc::now().timestamp_millis(),
            state: "active".to_string(),
            metadata: serde_json::json!({}),
        };

        // Store session
        storage.store_session(session.clone()).await.unwrap();

        // Get session
        let retrieved = storage.get_session("session-1").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "session-1");
    }
}
