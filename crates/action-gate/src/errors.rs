//! Error types for gate validation

use thiserror::Error;

/// Gate validation error enumeration
#[derive(Debug, Error, Clone)]
pub enum GateError {
    /// Validation timeout
    #[error("Validation timeout after {0}ms")]
    Timeout(u64),

    /// Condition evaluation failed
    #[error("Condition evaluation failed: {0}")]
    ConditionFailed(String),

    /// Invalid expectation spec
    #[error("Invalid ExpectSpec: {0}")]
    InvalidSpec(String),

    /// Missing required signal
    #[error("Missing required signal: {0}")]
    MissingSignal(String),

    /// CDP communication error
    #[error("CDP error: {0}")]
    CdpError(String),

    /// Evidence collection failed
    #[error("Evidence collection failed: {0}")]
    EvidenceFailed(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

impl GateError {
    /// Check if error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(self, GateError::Timeout(_) | GateError::CdpError(_))
    }

    /// Get error severity (0=low, 1=medium, 2=high, 3=critical)
    pub fn severity(&self) -> u8 {
        match self {
            GateError::Internal(_) => 3,
            GateError::CdpError(_) | GateError::InvalidSpec(_) => 2,
            GateError::Timeout(_) | GateError::ConditionFailed(_) => 1,
            _ => 0,
        }
    }
}
