//! Error types for action primitives

use thiserror::Error;

/// Comprehensive error types for action primitive operations
#[derive(Debug, Error, Clone)]
pub enum ActionError {
    /// Navigation timed out waiting for page load
    #[error("Navigation timeout: {0}")]
    NavTimeout(String),

    /// Wait operation timed out
    #[error("Wait timeout: {0}")]
    WaitTimeout(String),

    /// Operation was cancelled or interrupted
    #[error("Operation interrupted: {0}")]
    Interrupted(String),

    /// Element is not clickable (obscured, disabled, or not interactable)
    #[error("Element not clickable: {0}")]
    NotClickable(String),

    /// Element is not enabled for interaction
    #[error("Element not enabled: {0}")]
    NotEnabled(String),

    /// Dropdown option was not found
    #[error("Option not found in dropdown: {0}")]
    OptionNotFound(String),

    /// Element anchor could not be resolved
    #[error("Anchor not found: {0}")]
    AnchorNotFound(String),

    /// Scroll target is invalid or unreachable
    #[error("Scroll target invalid: {0}")]
    ScrollTargetInvalid(String),

    /// Execution route became stale (frame navigation or reload)
    #[error("Stale route: {0}")]
    StaleRoute(String),

    /// CDP communication or protocol error
    #[error("CDP I/O error: {0}")]
    CdpIo(String),

    /// Policy denied the operation
    #[error("Policy denied: {0}")]
    PolicyDenied(String),

    /// Internal error (should not happen in normal operation)
    #[error("Internal error: {0}")]
    Internal(String),
}

impl ActionError {
    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ActionError::WaitTimeout(_) | ActionError::NotClickable(_) | ActionError::CdpIo(_)
        )
    }

    /// Get error severity level (0=low, 1=medium, 2=high, 3=critical)
    pub fn severity(&self) -> u8 {
        match self {
            ActionError::Internal(_) | ActionError::StaleRoute(_) => 3,
            ActionError::NavTimeout(_) | ActionError::PolicyDenied(_) | ActionError::CdpIo(_) => 2,
            ActionError::WaitTimeout(_)
            | ActionError::AnchorNotFound(_)
            | ActionError::NotEnabled(_) => 1,
            _ => 0,
        }
    }
}
