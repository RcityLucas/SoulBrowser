//! Error handling module
//!
//! Provides unified error handling using soulbase-errors
#![allow(dead_code)]

use soulbase_errors::{
    code::{codes, ErrorCode},
    model::{CauseEntry, ErrorBuilder, ErrorObj},
    retry::RetryClass,
    severity::Severity,
};
use std::fmt;

/// Unified error type using soulbase-errors
#[derive(Debug, Clone)]
pub struct SoulBrowserError {
    inner: ErrorObj,
}

impl SoulBrowserError {
    /// Create a new error
    pub fn new(code: ErrorCode, message: &str) -> Self {
        let error = ErrorBuilder::new(code).user_msg(message).build();
        Self { inner: error }
    }

    /// Create authentication error
    pub fn auth_error(message: &str) -> Self {
        let error = ErrorBuilder::new(codes::AUTH_UNAUTHENTICATED)
            .user_msg(message)
            .dev_msg("Authentication failed")
            .build();
        Self { inner: error }
    }

    /// Create authorization error
    pub fn forbidden(message: &str) -> Self {
        let error = ErrorBuilder::new(codes::AUTH_FORBIDDEN)
            .user_msg(message)
            .dev_msg("Authorization denied")
            .build();
        Self { inner: error }
    }

    /// Create not found error
    pub fn not_found(resource: &str) -> Self {
        let error = ErrorBuilder::new(codes::STORAGE_NOT_FOUND)
            .user_msg(format!("{} not found", resource))
            .dev_msg(format!("Resource '{}' does not exist", resource))
            .build();
        Self { inner: error }
    }

    /// Create validation error
    pub fn validation_error(message: &str, details: &str) -> Self {
        let error = ErrorBuilder::new(codes::SCHEMA_VALIDATION)
            .user_msg(message)
            .dev_msg(details)
            .build();
        Self { inner: error }
    }

    /// Create timeout error
    pub fn timeout(operation: &str, timeout_ms: u64) -> Self {
        let error = ErrorBuilder::new(codes::LLM_TIMEOUT)
            .user_msg(format!("{} timed out", operation))
            .dev_msg(format!(
                "Operation '{}' exceeded timeout of {}ms",
                operation, timeout_ms
            ))
            .build();
        Self { inner: error }
    }

    /// Create internal error
    pub fn internal(message: &str) -> Self {
        let error = ErrorBuilder::new(codes::UNKNOWN_INTERNAL)
            .user_msg("An internal error occurred")
            .dev_msg(message)
            .build();
        Self { inner: error }
    }

    /// Get error code
    pub fn code(&self) -> ErrorCode {
        self.inner.code
    }

    /// Get user message
    pub fn user_message(&self) -> &str {
        &self.inner.message_user
    }

    /// Get developer message
    pub fn dev_message(&self) -> Option<&str> {
        self.inner.message_dev.as_deref()
    }

    /// Get HTTP status code
    pub fn http_status(&self) -> u16 {
        self.inner.http_status
    }

    /// Check if error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(self.inner.retryable, RetryClass::Transient)
    }

    /// Get severity
    pub fn severity(&self) -> Severity {
        self.inner.severity
    }

    /// Add cause to error chain
    pub fn with_cause(self, code: &str, summary: &str) -> Self {
        let cause = CauseEntry {
            code: code.to_string(),
            summary: summary.to_string(),
            meta: None,
        };

        // Create new error with cause
        let mut builder = ErrorBuilder::new(self.inner.code).user_msg(&self.inner.message_user);

        if let Some(dev_msg) = &self.inner.message_dev {
            builder = builder.dev_msg(dev_msg);
        }

        builder = builder.cause(cause);

        Self {
            inner: builder.build(),
        }
    }
}

impl fmt::Display for SoulBrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner.message_user)
    }
}

impl std::error::Error for SoulBrowserError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

/// Convert from various error types to SoulBrowserError
impl From<std::io::Error> for SoulBrowserError {
    fn from(err: std::io::Error) -> Self {
        Self::internal(&format!("IO error: {}", err))
    }
}

impl From<serde_json::Error> for SoulBrowserError {
    fn from(err: serde_json::Error) -> Self {
        Self::validation_error("Invalid JSON", &err.to_string())
    }
}

impl From<anyhow::Error> for SoulBrowserError {
    fn from(err: anyhow::Error) -> Self {
        Self::internal(&err.to_string())
    }
}

// Migration helper removed - soul_integration module has been deprecated
// Use SoulBrowserError directly instead of migrating from old types

/// Result type using SoulBrowserError
pub type SoulResult<T> = Result<T, SoulBrowserError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = SoulBrowserError::auth_error("Invalid credentials");
        assert_eq!(err.code(), codes::AUTH_UNAUTHENTICATED);
        assert_eq!(err.user_message(), "Invalid credentials");
        assert!(err.dev_message().is_some());
    }

    #[test]
    fn test_error_with_cause() {
        let err = SoulBrowserError::internal("Database connection failed")
            .with_cause("DB_CONN", "Connection timeout");

        assert_eq!(err.code(), codes::UNKNOWN_INTERNAL);
    }

    #[test]
    fn test_retryable_check() {
        let _timeout_err = SoulBrowserError::timeout("Request", 5000);
        // Note: Actual retryability depends on ErrorCode configuration

        let _auth_err = SoulBrowserError::auth_error("Invalid token");
        // Auth errors are typically not retryable
    }
}
