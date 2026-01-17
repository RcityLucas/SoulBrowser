use thiserror::Error;

#[derive(Debug, Error)]
pub enum PrivacyError {
    #[error("privacy engine disabled for this context")]
    Disabled,
    #[error("unsupported payload for redaction: {0}")]
    Unsupported(&'static str),
    #[error("redaction policy prevented processing")]
    PolicyDenied,
    #[error("internal error: {0}")]
    Internal(String),
}

pub type PrivacyResult<T> = Result<T, PrivacyError>;
