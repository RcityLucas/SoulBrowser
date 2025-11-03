use thiserror::Error;

use soulbrowser_core_types::SoulError;

#[derive(Clone, Debug, Error)]
pub enum EsErrorKind {
    #[error("append rejected: {0}")]
    AppendRejected(String),
    #[error("hot capacity exceeded")]
    HotCapacityExceeded,
    #[error("cold write failed: {0}")]
    ColdWriteFailed(String),
    #[error("cold read failed: {0}")]
    ColdReadFailed(String),
    #[error("invalid filter: {0}")]
    InvalidFilter(String),
    #[error("requested range too large")]
    RangeTooLarge,
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Clone, Debug, Error)]
#[error(transparent)]
pub struct EsError(pub EsErrorKind);

impl EsError {
    pub fn new(kind: EsErrorKind) -> Self {
        Self(kind)
    }

    pub fn kind(&self) -> &EsErrorKind {
        &self.0
    }
}

impl From<EsError> for SoulError {
    fn from(value: EsError) -> Self {
        SoulError::new(value.to_string())
    }
}

impl From<EsErrorKind> for EsError {
    fn from(kind: EsErrorKind) -> Self {
        EsError(kind)
    }
}
