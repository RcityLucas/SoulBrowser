use serde_json::Value;

use super::{HostApi, HostContext, NoopHostApi};

pub trait InsightApi {
    fn emit(&self, ctx: &HostContext, insight: Value);
}

impl<T> InsightApi for T
where
    T: HostApi + ?Sized,
{
    fn emit(&self, ctx: &HostContext, insight: Value) {
        HostApi::emit_insight(self, ctx, insight)
    }
}

pub type DefaultInsightApi = NoopHostApi;
