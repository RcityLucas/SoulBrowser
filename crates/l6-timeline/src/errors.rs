use thiserror::Error;

#[derive(Debug, Error)]
pub enum TlError {
    #[error("invalid argument: {0}")]
    InvalidArg(String),
    #[error("policy denied request")]
    PolicyDenied,
    #[error("requested time range exceeds policy limits")]
    RangeTooLarge,
    #[error("payload exceeds maximum line budget after redaction")]
    Oversize,
    #[error("source data incomplete or stale")]
    StaleSource,
    #[error("export interrupted by caller or cancellation")]
    Interrupted,
    #[error("timeline feature not ready: {0}")]
    NotReady(&'static str),
    #[error("internal error: {0}")]
    Internal(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type TlResult<T> = Result<T, TlError>;
