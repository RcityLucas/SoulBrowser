///! Error types for semantic perceiver
use thiserror::Error;

/// Errors that can occur during semantic analysis
#[derive(Debug, Error)]
pub enum SemanticError {
    #[error("Content analysis failed: {0}")]
    AnalysisFailed(String),

    #[error("Classification failed: {0}")]
    ClassificationFailed(String),

    #[error("Summarization failed: {0}")]
    SummarizationFailed(String),

    #[error("Intent extraction failed: {0}")]
    IntentExtractionFailed(String),

    #[error("Language detection failed: {0}")]
    LanguageDetectionFailed(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Structural perceiver error: {0}")]
    StructuralError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for semantic operations
pub type Result<T> = std::result::Result<T, SemanticError>;
