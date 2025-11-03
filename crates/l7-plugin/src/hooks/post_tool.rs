use serde_json::Value;

use super::{HookCtx, HookExecutor};
use crate::errors::PluginResult;
use crate::manifest::PluginManifest;
use std::sync::Arc;

pub async fn invoke_post_tool<E: HookExecutor + ?Sized>(
    executor: &E,
    manifest: Arc<PluginManifest>,
    payload: Value,
    ctx: HookCtx,
) -> PluginResult<Value> {
    executor.invoke(manifest, "post_tool", payload, ctx).await
}
