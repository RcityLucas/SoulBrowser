use serde_json::Value;

use super::{HostApi, HostContext, NoopHostApi};

pub trait KvApi {
    fn put(&self, ctx: &HostContext, key: &str, value: Value) -> Result<(), String>;
    fn get(&self, ctx: &HostContext, key: &str) -> Result<Option<Value>, String>;
}

impl<T> KvApi for T
where
    T: HostApi + ?Sized,
{
    fn put(&self, ctx: &HostContext, key: &str, value: Value) -> Result<(), String> {
        HostApi::kv_put(self, ctx, key, value)
    }

    fn get(&self, ctx: &HostContext, key: &str) -> Result<Option<Value>, String> {
        HostApi::kv_get(self, ctx, key)
    }
}

pub type DefaultKvApi = NoopHostApi;
