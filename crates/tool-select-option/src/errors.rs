use soulbrowser_core_types::SoulError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SelectError {
    #[error("tool disabled by policy")]
    Disabled,
    #[error("mode not permitted")]
    ModeNotAllowed,
    #[error("field is readonly")]
    ReadOnly,
    #[error("field disabled")]
    DisabledField,
    #[error("match kind not supported")]
    MatchNotSupported,
    #[error("option not found for target")]
    OptionMissing,
    #[error("invalid selection target: {0}")]
    InvalidTarget(String),
    #[error("precheck failed: {0}")]
    Precheck(String),
    #[error("self heal unavailable")]
    SelfHealUnavailable,
    #[error("operation cancelled")]
    Cancelled,
}

impl From<SelectError> for SoulError {
    fn from(err: SelectError) -> Self {
        SoulError::new(err.to_string())
    }
}
