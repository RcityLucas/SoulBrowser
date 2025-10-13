#![allow(dead_code)]

use std::fmt;

use thiserror::Error;
use uuid::Uuid;

/// Shared error type stub for the L1 unified kernel crates.
#[derive(Debug, Error, Clone)]
pub enum SoulError {
    #[error("{message}")]
    Message { message: String },
}

impl SoulError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Message {
            message: message.into(),
        }
    }
}

#[cfg_attr(feature = "serde-full", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

#[cfg_attr(feature = "serde-full", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PageId(pub String);

impl PageId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

#[cfg_attr(feature = "serde-full", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct FrameId(pub String);

impl FrameId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

#[cfg_attr(feature = "serde-full", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ActionId(pub String);

impl ActionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

#[cfg_attr(feature = "serde-full", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

#[cfg_attr(feature = "serde-full", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoutePrefer {
    Focused,
    RecentNav,
    MainFrame,
}

#[cfg_attr(feature = "serde-full", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RoutingHint {
    pub session: Option<SessionId>,
    pub page: Option<PageId>,
    pub frame: Option<FrameId>,
    pub prefer: Option<RoutePrefer>,
}

#[cfg_attr(feature = "serde-full", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecRoute {
    pub session: SessionId,
    pub page: PageId,
    pub frame: FrameId,
    pub mutex_key: String,
}

impl ExecRoute {
    pub fn new(session: SessionId, page: PageId, frame: FrameId) -> Self {
        let mutex_key = format!("frame:{}", frame.0);
        Self {
            session,
            page,
            frame,
            mutex_key,
        }
    }
}

impl fmt::Display for ExecRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "session={} page={} frame={} mutex={}",
            self.session.0, self.page.0, self.frame.0, self.mutex_key
        )
    }
}

/// Placeholder ToolCall representation; to be replaced with the full struct during Phase 2.
#[cfg(feature = "serde-full")]
pub type ToolPayload = serde_json::Value;

#[cfg(not(feature = "serde-full"))]
pub type ToolPayload = ();

#[cfg_attr(feature = "serde-full", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolCall {
    pub call_id: Option<String>,
    pub task_id: Option<TaskId>,
    pub tool: String,
    pub payload: ToolPayload,
}
#[cfg(feature = "serde-full")]
use serde_json;
