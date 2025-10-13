use soulbrowser_core_types::SoulError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("not implemented: {0}")]
    NotImplemented(String),
    #[error("invalid policy: {0}")]
    Invalid(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("unsupported policy path: {0}")]
    UnsupportedPath(String),
    #[error("invalid value: {0}")]
    InvalidValue(String),
}

impl From<PolicyError> for SoulError {
    fn from(value: PolicyError) -> Self {
        SoulError::new(value.to_string())
    }
}
