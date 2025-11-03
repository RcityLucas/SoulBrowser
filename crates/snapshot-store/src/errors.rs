use thiserror::Error;

use soulbrowser_core_types::SoulError;

#[derive(Clone, Debug, Error)]
pub enum SnapErrKind {
    #[error("snapshot store disabled")]
    Disabled,
    #[error("oversize payload rejected")]
    Oversize,
    #[error("quota exceeded")]
    QuotaExceeded,
    #[error("ttl too long")]
    TtlTooLong,
    #[error("io failure: {0}")]
    IoFailed(String),
    #[error("snapshot not found")]
    NotFound,
    #[error("snapshot corrupt")]
    Corrupt,
    #[error("snapshot store read-only")]
    ReadOnly,
    #[error("privacy violation: {0}")]
    PrivacyViolation(String),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Clone, Debug, Error)]
#[error(transparent)]
pub struct SnapError(pub SnapErrKind);

impl SnapError {
    pub fn new(kind: SnapErrKind) -> Self {
        Self(kind)
    }

    pub fn kind(&self) -> &SnapErrKind {
        &self.0
    }
}

impl From<SnapError> for SoulError {
    fn from(value: SnapError) -> Self {
        SoulError::new(value.to_string())
    }
}

impl From<SnapErrKind> for SnapError {
    fn from(kind: SnapErrKind) -> Self {
        SnapError(kind)
    }
}
