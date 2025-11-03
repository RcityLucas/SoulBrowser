use serde_json::Value;

use super::{HookCtx, HookExecutor};
use crate::errors::PluginResult;
use crate::manifest::PluginManifest;
use std::sync::Arc;

pub async fn invoke_on_span<E: HookExecutor + ?Sized>(
    executor: &E,
    manifest: Arc<PluginManifest>,
    payload: Value,
    ctx: HookCtx,
) -> PluginResult<Value> {
    executor.invoke(manifest, "on_span", payload, ctx).await
}
