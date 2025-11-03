///! Semantic Perceiver - Content understanding and text analysis
///!
///! This crate provides semantic analysis capabilities for web content:
///! - Content type classification (article, product, form, etc.)
///! - Page intent detection (informational, transactional, etc.)
///! - Language detection
///! - Text summarization
///! - Keyword extraction
///! - Readability analysis
pub mod classifier;
pub mod errors;
pub mod keywords;
pub mod language;
pub mod models;
pub mod semantic;
pub mod summarizer;

// Re-exports
pub use classifier::Classifier;
pub use errors::{Result, SemanticError};
pub use keywords::KeywordExtractor;
pub use language::LanguageDetector;
pub use models::*;
pub use semantic::{SemanticPerceiver, SemanticPerceiverImpl};
pub use summarizer::Summarizer;
