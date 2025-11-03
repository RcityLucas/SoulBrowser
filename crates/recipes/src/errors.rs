use thiserror::Error;

use soulbrowser_core_types::SoulError;

#[derive(Clone, Debug, Error)]
pub enum RecErrorKind {
    #[error("recipes module disabled")]
    Disabled,
    #[error("capacity exceeded")]
    CapacityExceeded,
    #[error("privacy violation: {0}")]
    PrivacyViolation(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("recipe not found")]
    NotFound,
    #[error("conflict with active recipe")]
    Conflict,
    #[error("external dependency unavailable")]
    ExternalUnavailable,
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Clone, Debug, Error)]
#[error(transparent)]
pub struct RecError(pub RecErrorKind);

impl RecError {
    pub fn new(kind: RecErrorKind) -> Self {
        Self(kind)
    }

    pub fn kind(&self) -> &RecErrorKind {
        &self.0
    }
}

impl From<RecError> for SoulError {
    fn from(value: RecError) -> Self {
        SoulError::new(value.to_string())
    }
}

impl From<RecErrorKind> for RecError {
    fn from(kind: RecErrorKind) -> Self {
        RecError(kind)
    }
}
