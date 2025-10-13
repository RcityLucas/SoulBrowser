//! SoulBrowser L0 extensions bridge scaffold.
//!
//! The production implementation will manage allowlists, establish channels with MV3 extensions,
//! and forward invoke requests with policy/permission checks. This placeholder describes the core
//! interfaces so consumers can experiment with the integration points while the logic is authored.

pub mod config;

use async_trait::async_trait;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Extension identifier wrapper.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ExtensionId(pub String);

/// Logical channel identifier.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub Uuid);

impl ChannelId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Scope of the channel.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Scope {
    Tab,
    Background,
}

/// Request envelope sent to extensions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BridgeRequest {
    pub req_id: Uuid,
    pub op: String,
    pub payload: serde_json::Value,
    pub deadline_ms: u64,
}

/// Response from extensions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BridgeResponse {
    pub req_id: Uuid,
    pub ok: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Errors surfaced by the bridge.
#[derive(Clone, Debug, Error)]
pub enum BridgeError {
    #[error("unsupported environment")]
    Unsupported,
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    #[error("permission required: {0}")]
    PermissionRequired(String),
    #[error("timeout")]
    Timeout,
    #[error("channel closed")]
    ChannelClosed,
    #[error("internal error: {0}")]
    Internal(String),
}

/// Channel event bus placeholder.
pub type BridgeEventBus = broadcast::Sender<BridgeEvent>;

/// Events emitted by the bridge to observers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BridgeEvent {
    BridgeReady {
        extensions: Vec<ExtensionId>,
    },
    ChannelOpen {
        extension: ExtensionId,
        scope: Scope,
        channel: ChannelId,
    },
    ChannelClosed {
        extension: ExtensionId,
        scope: Scope,
        channel: ChannelId,
    },
    InvokeOk {
        extension: ExtensionId,
        op: String,
    },
    InvokeFail {
        extension: ExtensionId,
        op: String,
        error: String,
    },
}

#[async_trait]
pub trait Bridge {
    async fn enable_bridge(&self) -> Result<(), BridgeError>;
    async fn disable_bridge(&self) -> Result<(), BridgeError>;
    async fn open_channel(
        &self,
        extension: ExtensionId,
        scope: Scope,
    ) -> Result<ChannelId, BridgeError>;
    async fn invoke(
        &self,
        extension: ExtensionId,
        scope: Scope,
        request: BridgeRequest,
    ) -> Result<BridgeResponse, BridgeError>;
}

/// Placeholder implementation; all operations return an error until the feature lands.
pub struct ExtensionsBridge {
    pub events: BridgeEventBus,
    allowed: Vec<ExtensionId>,
    enabled: AtomicBool,
    channels: DashMap<ChannelId, ChannelState>,
}

#[derive(Clone, Debug)]
struct ChannelState {
    extension: ExtensionId,
    scope: Scope,
}

impl ExtensionsBridge {
    pub fn new(events: BridgeEventBus, allowed: Vec<ExtensionId>) -> Arc<Self> {
        Arc::new(Self {
            events,
            allowed,
            enabled: AtomicBool::new(false),
            channels: DashMap::new(),
        })
    }

    fn is_allowed(&self, extension: &ExtensionId) -> bool {
        self.allowed.iter().any(|e| e == extension)
    }
}

#[async_trait]
impl Bridge for ExtensionsBridge {
    async fn enable_bridge(&self) -> Result<(), BridgeError> {
        if self.enabled.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _ = self.events.send(BridgeEvent::BridgeReady {
            extensions: self.allowed.clone(),
        });
        Ok(())
    }

    async fn disable_bridge(&self) -> Result<(), BridgeError> {
        if !self.enabled.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        let mut pending = Vec::new();
        for entry in self.channels.iter() {
            pending.push((
                entry.key().clone(),
                entry.value().extension.clone(),
                entry.value().scope,
            ));
        }
        self.channels.clear();
        for (channel_id, extension, scope) in pending {
            let _ = self.events.send(BridgeEvent::ChannelClosed {
                extension,
                scope,
                channel: channel_id,
            });
        }
        Ok(())
    }

    async fn open_channel(
        &self,
        _extension: ExtensionId,
        _scope: Scope,
    ) -> Result<ChannelId, BridgeError> {
        if !self.enabled.load(Ordering::SeqCst) {
            return Err(BridgeError::Unsupported);
        }

        if !self.is_allowed(&_extension) {
            return Err(BridgeError::PolicyDenied(format!(
                "extension {} not in allowlist",
                _extension.0
            )));
        }

        let channel_id = ChannelId::new();
        let channel_clone = channel_id.clone();
        self.channels.insert(
            channel_id.clone(),
            ChannelState {
                extension: _extension.clone(),
                scope: _scope,
            },
        );

        let _ = self.events.send(BridgeEvent::ChannelOpen {
            extension: _extension,
            scope: _scope,
            channel: channel_id.clone(),
        });

        Ok(channel_clone)
    }

    async fn invoke(
        &self,
        _extension: ExtensionId,
        _scope: Scope,
        _request: BridgeRequest,
    ) -> Result<BridgeResponse, BridgeError> {
        let channel = self
            .channels
            .iter()
            .find(|entry| entry.value().extension == _extension && entry.value().scope == _scope)
            .map(|entry| entry.key().clone())
            .ok_or(BridgeError::ChannelClosed)?;

        let _ = self.events.send(BridgeEvent::InvokeOk {
            extension: _extension,
            op: _request.op.clone(),
        });

        Ok(BridgeResponse {
            req_id: _request.req_id,
            ok: true,
            data: Some(serde_json::json!({"channel": channel.0.to_string()})),
            error: None,
        })
    }
}
