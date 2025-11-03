use thiserror::Error;

use soulbrowser_core_types::SoulError;

#[derive(Debug, Error)]
pub enum TypeTextError {
    #[error("tool disabled by policy")]
    Disabled,
    #[error("text exceeds max length ({0})")]
    TextTooLong(usize),
    #[error("field is readonly")]
    ReadOnly,
    #[error("field disabled")]
    DisabledField,
    #[error("mode not allowed")]
    ModeNotAllowed,
    #[error("paste requires permission")]
    PasteDenied,
    #[error("precheck failed: {0}")]
    Precheck(String),
    #[error("self heal unavailable")]
    SelfHealUnavailable,
    #[error("operation cancelled")]
    Cancelled,
}

impl From<TypeTextError> for SoulError {
    fn from(err: TypeTextError) -> Self {
        SoulError::new(err.to_string())
    }
}
