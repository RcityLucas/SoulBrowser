///! Error types for perception hub

use thiserror::Error;

/// Errors that can occur in the perception hub
#[derive(Debug, Error)]
pub enum HubError {
    #[error("Structural perceiver error: {0}")]
    Structural(String),

    #[error("Visual perceiver error: {0}")]
    Visual(String),

    #[error("Semantic perceiver error: {0}")]
    Semantic(String),

    #[error("Multi-modal fusion error: {0}")]
    Fusion(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Perceiver not available: {0}")]
    PerceiverUnavailable(String),

    #[error("Analysis timeout: {0}")]
    Timeout(String),
}

/// Result type for hub operations
pub type Result<T> = std::result::Result<T, HubError>;

// Implement conversions from perceiver errors
impl From<perceiver_structural::errors::PerceiverError> for HubError {
    fn from(err: perceiver_structural::errors::PerceiverError) -> Self {
        HubError::Structural(format!("{:?}", err))
    }
}

impl From<perceiver_visual::VisualError> for HubError {
    fn from(err: perceiver_visual::VisualError) -> Self {
        HubError::Visual(format!("{:?}", err))
    }
}

impl From<perceiver_semantic::SemanticError> for HubError {
    fn from(err: perceiver_semantic::SemanticError) -> Self {
        HubError::Semantic(format!("{:?}", err))
    }
}
