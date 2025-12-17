//! Domain types and models for SoulBrowser
//!
//! Core domain types leveraging soulbase-types for the soul-base ecosystem
#![allow(dead_code)]

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
pub use soulbase_types::tenant::TenantId;
use soulbase_types::{
    envelope::Envelope,
    id::{CorrelationId, Id},
    subject::{Subject, SubjectKind},
    time::Timestamp,
    trace::TraceContext,
};
use std::collections::HashMap;

/// Browser type enumeration
#[derive(Clone, Debug, Default, Serialize, Deserialize, ValueEnum)]
pub enum BrowserType {
    #[default]
    Chromium,
    Chrome,
    Firefox,
    Safari,
    Edge,
}

/// Browser action using soulbase types
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserAction {
    pub id: Id,
    pub timestamp: Timestamp,
    pub action_type: ActionType,
    pub target: Option<String>,
    pub value: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ActionType {
    Navigate,
    Click,
    Type,
    Screenshot,
    WaitForElement,
    Execute,
    Extract,
    Scroll,
    KeyPress,
}

impl BrowserAction {
    /// Create a new browser action
    pub fn new(action_type: ActionType) -> Self {
        Self {
            id: Id::new_random(),
            timestamp: Timestamp(chrono::Utc::now().timestamp_millis()),
            action_type,
            target: None,
            value: None,
            metadata: HashMap::new(),
        }
    }

    /// Set target for the action
    pub fn with_target(mut self, target: String) -> Self {
        self.target = Some(target);
        self
    }

    /// Set value for the action
    pub fn with_value(mut self, value: String) -> Self {
        self.value = Some(value);
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: String, value: serde_json::Value) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// Convert to Envelope for soul-base messaging
    pub fn to_envelope(&self, subject: Subject) -> Envelope<BrowserAction> {
        Envelope::new(
            self.id.clone(),
            self.timestamp,
            format!("browser-action-{}", self.id.0),
            subject,
            "1.0.0",
            self.clone(),
        )
    }
}

/// Browser session using soulbase types
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserSession {
    pub id: Id,
    pub tenant: TenantId,
    pub subject: Subject,
    pub created_at: Timestamp,
    pub correlation_id: CorrelationId,
    pub state: SessionState,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SessionState {
    Active,
    Paused,
    Completed,
    Failed,
}

impl BrowserSession {
    /// Create a new browser session
    pub fn new(tenant_id: String, _user_id: String) -> Self {
        let subject = Subject {
            kind: SubjectKind::User,
            subject_id: Id::new_random(),
            tenant: TenantId(tenant_id.clone()),
            claims: Default::default(),
        };

        Self {
            id: Id::new_random(),
            tenant: TenantId(tenant_id),
            subject,
            created_at: Timestamp(chrono::Utc::now().timestamp_millis()),
            correlation_id: CorrelationId(uuid::Uuid::new_v4().to_string()),
            state: SessionState::Active,
            metadata: HashMap::new(),
        }
    }

    /// Set session state
    pub fn set_state(&mut self, state: SessionState) {
        self.state = state;
    }

    /// Add metadata
    pub fn add_metadata(&mut self, key: String, value: serde_json::Value) {
        self.metadata.insert(key, value);
    }

    /// Create an action context for this session
    #[allow(dead_code)]
    pub fn create_action_context(&self) -> ActionContext {
        ActionContext {
            session_id: self.id.clone(),
            tenant: self.tenant.clone(),
            subject: self.subject.clone(),
            correlation_id: self.correlation_id.clone(),
            timestamp: Timestamp(chrono::Utc::now().timestamp_millis()),
        }
    }
}

/// Action context using soulbase types
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ActionContext {
    pub session_id: Id,
    pub tenant: TenantId,
    pub subject: Subject,
    pub correlation_id: CorrelationId,
    pub timestamp: Timestamp,
}

impl ActionContext {
    /// Create trace context for distributed tracing
    #[allow(dead_code)]
    pub fn create_trace_context(&self) -> TraceContext {
        TraceContext {
            trace_id: Some(uuid::Uuid::new_v4().to_string()),
            span_id: Some(uuid::Uuid::new_v4().to_string()),
            baggage: Default::default(),
        }
    }
}

/// Migration helper to convert old types to soulbase types
pub mod migration {

    /// Example migration function - to be implemented when replacing actual code
    #[allow(dead_code)]
    pub fn example_migration() {
        // This module will contain the actual migration functions
        // when we start replacing the old soul_integration types
        // with the new soulbase-types based ones
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browser_action() {
        let action = BrowserAction::new(ActionType::Navigate)
            .with_target("https://example.com".to_string())
            .with_metadata(
                "user_agent".to_string(),
                serde_json::Value::String("Chrome".to_string()),
            );

        assert!(action.target.is_some());
        assert_eq!(action.target.unwrap(), "https://example.com");
        assert!(action.metadata.contains_key("user_agent"));
    }

    #[test]
    fn test_browser_session() {
        let mut session = BrowserSession::new("test-tenant".to_string(), "user-123".to_string());

        assert_eq!(session.tenant.0, "test-tenant");
        assert!(matches!(session.state, SessionState::Active));

        session.set_state(SessionState::Completed);
        assert!(matches!(session.state, SessionState::Completed));
    }

    #[test]
    fn test_action_envelope() {
        let action = BrowserAction::new(ActionType::Click).with_target("#button".to_string());

        let subject = Subject {
            kind: SubjectKind::User,
            subject_id: Id::new_random(),
            tenant: TenantId("test".to_string()),
            claims: Default::default(),
        };

        let envelope = action.to_envelope(subject);
        assert_eq!(envelope.schema_ver, "1.0.0");
        assert_eq!(envelope.payload.action_type as u8, ActionType::Click as u8);
    }
}
