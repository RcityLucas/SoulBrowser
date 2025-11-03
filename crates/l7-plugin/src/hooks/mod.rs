pub mod on_export_line;
pub mod on_span;
pub mod post_tool;
pub mod pre_tool;

use crate::errors::PluginResult;
use crate::manifest::PluginManifest;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct HookCtx {
    pub tenant: Option<String>,
    pub action_id: Option<String>,
    pub trace_id: Option<String>,
}

impl Default for HookCtx {
    fn default() -> Self {
        Self {
            tenant: None,
            action_id: None,
            trace_id: None,
        }
    }
}

#[async_trait]
pub trait HookExecutor: Send + Sync {
    async fn invoke(
        &self,
        manifest: Arc<PluginManifest>,
        hook: &str,
        payload: Value,
        ctx: HookCtx,
    ) -> PluginResult<Value>;
}
