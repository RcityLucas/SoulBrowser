use serde_json::Value;

use super::{HostApi, HostContext, NoopHostApi};

pub trait ToolPatchApi {
    fn patch(&self, ctx: &HostContext, patch: Value) -> Result<(), String>;
}

impl<T> ToolPatchApi for T
where
    T: HostApi + ?Sized,
{
    fn patch(&self, ctx: &HostContext, patch: Value) -> Result<(), String> {
        HostApi::tool_patch(self, ctx, patch)
    }
}

pub type DefaultToolPatchApi = NoopHostApi;
