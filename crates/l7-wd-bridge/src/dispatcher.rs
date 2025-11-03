use async_trait::async_trait;
use serde_json::Value;
use soulbrowser_core_types::{RoutingHint, ToolCall as L1ToolCall};

use crate::errors::{BridgeError, BridgeResult};

#[async_trait]
pub trait ToolDispatcher: Send + Sync {
    async fn run_tool(
        &self,
        tenant: &str,
        call: L1ToolCall,
        routing: Option<RoutingHint>,
    ) -> BridgeResult<Value>;
}

pub struct NoopDispatcher;

#[async_trait]
impl ToolDispatcher for NoopDispatcher {
    async fn run_tool(
        &self,
        _tenant: &str,
        _call: L1ToolCall,
        _routing: Option<RoutingHint>,
    ) -> BridgeResult<Value> {
        Err(BridgeError::NotImplemented)
    }
}
