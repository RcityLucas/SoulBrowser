use thiserror::Error;

/// Errors emitted by the agent-core crate.
#[derive(Debug, Error)]
pub enum AgentError {
    /// Raised when an agent request is malformed or missing required fields.
    #[error("invalid agent request: {0}")]
    InvalidRequest(String),

    /// Raised when a plan contains an unsupported tool or step configuration.
    #[error("unsupported plan element: {0}")]
    UnsupportedPlan(String),

    /// Raised when converting the plan into an ActionFlow structure fails.
    #[error("failed to convert plan: {0}")]
    Conversion(String),
}

impl AgentError {
    /// Helper for wrapping static string errors.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }

    /// Helper for unsupported plan scenarios.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::UnsupportedPlan(message.into())
    }

    /// Helper for conversion-related failures.
    pub fn conversion(message: impl Into<String>) -> Self {
        Self::Conversion(message.into())
    }
}
