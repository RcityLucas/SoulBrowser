///! Data models for semantic analysis
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Content type classification
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContentType {
    /// Article or blog post
    Article,
    /// E-commerce product page
    Product,
    /// Form or input page
    Form,
    /// Navigation or directory page
    Navigation,
    /// Search results page
    Search,
    /// Social media content
    Social,
    /// Documentation page
    Documentation,
    /// Landing page
    Landing,
    /// Error page
    Error,
    /// Unknown or mixed content
    Unknown,
}

/// Page intent classification
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageIntent {
    /// Informational content
    Informational,
    /// Transactional (purchase, signup, etc.)
    Transactional,
    /// Navigational (menu, directory)
    Navigational,
    /// Interactive (tool, calculator, game)
    Interactive,
    /// Unknown intent
    Unknown,
}

/// Language information
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageInfo {
    /// ISO 639-1 language code (e.g., "en", "zh", "es")
    pub code: String,
    /// Human-readable language name
    pub name: String,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f64,
}

/// Text entity extraction result
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    /// Entity text
    pub text: String,
    /// Entity type (e.g., "person", "organization", "location")
    pub entity_type: String,
    /// Confidence score
    pub confidence: f64,
}

/// Content summary
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentSummary {
    /// Short summary (1-2 sentences)
    pub short: String,
    /// Medium summary (paragraph)
    pub medium: Option<String>,
    /// Key points extracted
    pub key_points: Vec<String>,
    /// Word count of original content
    pub word_count: usize,
}

/// Semantic analysis result
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticAnalysisResult {
    /// Content type classification
    pub content_type: ContentType,
    /// Page intent
    pub intent: PageIntent,
    /// Primary language
    pub language: LanguageInfo,
    /// Content summary
    pub summary: ContentSummary,
    /// Extracted entities
    pub entities: Vec<Entity>,
    /// Keywords with relevance scores
    pub keywords: HashMap<String, f64>,
    /// Sentiment analysis (-1.0 to 1.0, negative to positive)
    pub sentiment: Option<f64>,
    /// Readability score (0.0 to 100.0, higher is more readable)
    pub readability: Option<f64>,
}

/// Options for semantic analysis
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticOptions {
    /// Enable entity extraction
    pub extract_entities: bool,
    /// Enable keyword extraction
    pub extract_keywords: bool,
    /// Enable sentiment analysis
    pub analyze_sentiment: bool,
    /// Enable readability scoring
    pub analyze_readability: bool,
    /// Maximum number of keywords to extract
    pub max_keywords: usize,
    /// Minimum keyword relevance score
    pub min_keyword_score: f64,
}

impl Default for SemanticOptions {
    fn default() -> Self {
        Self {
            extract_entities: true,
            extract_keywords: true,
            analyze_sentiment: false,
            analyze_readability: true,
            max_keywords: 10,
            min_keyword_score: 0.3,
        }
    }
}

/// Text extraction options
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextExtractionOptions {
    /// Include hidden text
    pub include_hidden: bool,
    /// Include metadata (title, description)
    pub include_metadata: bool,
    /// Include alt text from images
    pub include_alt_text: bool,
    /// Include aria-label attributes
    pub include_aria_labels: bool,
}

impl Default for TextExtractionOptions {
    fn default() -> Self {
        Self {
            include_hidden: false,
            include_metadata: true,
            include_alt_text: true,
            include_aria_labels: true,
        }
    }
}

/// Extracted text content
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedText {
    /// Main body text
    pub body: String,
    /// Page title
    pub title: Option<String>,
    /// Meta description
    pub description: Option<String>,
    /// Headings in order
    pub headings: Vec<String>,
    /// Links with their text
    pub links: Vec<(String, String)>, // (text, url)
    /// Total character count
    pub char_count: usize,
}

impl ExtractedText {
    /// Get all text content concatenated
    pub fn all_text(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref title) = self.title {
            parts.push(title.clone());
        }

        if let Some(ref desc) = self.description {
            parts.push(desc.clone());
        }

        parts.extend(self.headings.clone());
        parts.push(self.body.clone());

        parts.join("\n\n")
    }
}
