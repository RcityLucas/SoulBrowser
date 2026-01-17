//! Error types for locator system

use thiserror::Error;

/// Locator error enumeration
#[derive(Debug, Error, Clone)]
pub enum LocatorError {
    /// Element not found with any strategy
    #[error("Element not found: {0}")]
    ElementNotFound(String),

    /// Multiple elements match (ambiguous)
    #[error("Multiple elements match: {0}")]
    AmbiguousMatch(String),

    /// Invalid anchor descriptor
    #[error("Invalid anchor: {0}")]
    InvalidAnchor(String),

    /// Strategy execution failed
    #[error("Strategy '{strategy}' failed: {reason}")]
    StrategyFailed { strategy: String, reason: String },

    /// CDP communication error
    #[error("CDP error: {0}")]
    CdpError(String),

    /// Timeout during resolution
    #[error("Resolution timeout: {0}")]
    Timeout(String),

    /// Heal attempt failed
    #[error("Heal failed: {0}")]
    HealFailed(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

impl LocatorError {
    /// Check if error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(self, LocatorError::Timeout(_) | LocatorError::CdpError(_))
    }

    /// Get error severity (0=low, 1=medium, 2=high, 3=critical)
    pub fn severity(&self) -> u8 {
        match self {
            LocatorError::Internal(_) => 3,
            LocatorError::CdpError(_) | LocatorError::Timeout(_) => 2,
            LocatorError::ElementNotFound(_)
            | LocatorError::HealFailed(_)
            | LocatorError::StrategyFailed { .. } => 1,
            _ => 0,
        }
    }
}
