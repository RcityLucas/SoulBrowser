use serde_json::Value;

use super::{HostApi, HostContext, NoopHostApi};

pub trait HistoryApi {
    fn query(&self, ctx: &HostContext, request: Value) -> Result<Value, String>;
    fn export(&self, ctx: &HostContext, request: Value) -> Result<Value, String>;
}

impl<T> HistoryApi for T
where
    T: HostApi + ?Sized,
{
    fn query(&self, ctx: &HostContext, request: Value) -> Result<Value, String> {
        HostApi::query_history(self, ctx, request)
    }

    fn export(&self, ctx: &HostContext, request: Value) -> Result<Value, String> {
        HostApi::export_timeline(self, ctx, request)
    }
}

pub type DefaultHistoryApi = NoopHostApi;
