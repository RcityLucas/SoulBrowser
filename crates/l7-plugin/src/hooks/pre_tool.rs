use serde_json::Value;

use super::{HookCtx, HookExecutor};
use crate::errors::{PluginError, PluginResult};
use crate::manifest::PluginManifest;
use std::sync::Arc;

pub async fn invoke_pre_tool<E: HookExecutor + ?Sized>(
    executor: &E,
    manifest: Arc<PluginManifest>,
    payload: Value,
    ctx: HookCtx,
) -> PluginResult<Value> {
    executor
        .invoke(manifest, "pre_tool", payload, ctx)
        .await
        .map_err(|err| match err {
            PluginError::Sandbox(_) => err,
            other => other,
        })
}
