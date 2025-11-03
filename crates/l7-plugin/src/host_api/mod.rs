pub mod history;
pub mod insight;
pub mod kv;
pub mod log;
pub mod tool_patch;

use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct HostContext {
    pub plugin: String,
    pub tenant: Option<String>,
}

pub trait HostApi: Send + Sync {
    fn log(&self, ctx: &HostContext, level: LogLevel, message: &str);
    fn emit_insight(&self, ctx: &HostContext, insight: Value);
    fn query_history(&self, ctx: &HostContext, request: Value) -> Result<Value, String>;
    fn export_timeline(&self, ctx: &HostContext, request: Value) -> Result<Value, String>;
    fn tool_patch(&self, ctx: &HostContext, patch: Value) -> Result<(), String>;
    fn kv_put(&self, ctx: &HostContext, key: &str, value: Value) -> Result<(), String>;
    fn kv_get(&self, ctx: &HostContext, key: &str) -> Result<Option<Value>, String>;
}

#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

pub struct NoopHostApi;

impl HostApi for NoopHostApi {
    fn log(&self, _ctx: &HostContext, _level: LogLevel, _message: &str) {}

    fn emit_insight(&self, _ctx: &HostContext, _insight: Value) {}

    fn query_history(&self, _ctx: &HostContext, _request: Value) -> Result<Value, String> {
        Err("history not available".into())
    }

    fn export_timeline(&self, _ctx: &HostContext, _request: Value) -> Result<Value, String> {
        Err("timeline not available".into())
    }

    fn tool_patch(&self, _ctx: &HostContext, _patch: Value) -> Result<(), String> {
        Err("patch not allowed".into())
    }

    fn kv_put(&self, _ctx: &HostContext, _key: &str, _value: Value) -> Result<(), String> {
        Err("kv disabled".into())
    }

    fn kv_get(&self, _ctx: &HostContext, _key: &str) -> Result<Option<Value>, String> {
        Ok(None)
    }
}
