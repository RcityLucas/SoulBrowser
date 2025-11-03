use thiserror::Error;

use soulbrowser_core_types::SoulError;

#[derive(Debug, Error)]
pub enum ClickError {
    #[error("tool disabled by policy")]
    Disabled,
    #[error("precheck failed: {0}")]
    Precheck(String),
    #[error("policy rejected button")]
    ButtonNotAllowed,
    #[error("offset out of range")]
    OffsetOutOfRange,
    #[error("self heal unavailable")]
    SelfHealUnavailable,
    #[error("operation cancelled")]
    Cancelled,
}

impl From<ClickError> for SoulError {
    fn from(err: ClickError) -> Self {
        SoulError::new(err.to_string())
    }
}
