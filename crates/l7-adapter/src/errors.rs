use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("adapter disabled")]
    Disabled,
    #[error("tenant not authorized")]
    UnauthorizedTenant,
    #[error("tenant not configured")]
    TenantNotFound,
    #[error("tool not permitted")]
    ToolNotAllowed,
    #[error("invalid argument")]
    InvalidArgument,
    #[error("rate limit exceeded")]
    TooManyRequests,
    #[error("concurrency limit reached")]
    ConcurrencyLimit,
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
    #[error("internal error")]
    Internal,
}

pub type AdapterResult<T> = Result<T, AdapterError>;
