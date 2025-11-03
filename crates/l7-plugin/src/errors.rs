use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("plugin not found: {0}")]
    NotFound(String),
    #[error("plugin blocked by policy")]
    Blocked,
    #[error("policy disabled")]
    Disabled,
    #[error("manifest error: {0}")]
    Manifest(String),
    #[error("sandbox error: {0}")]
    Sandbox(String),
}

pub type PluginResult<T> = Result<T, PluginError>;
