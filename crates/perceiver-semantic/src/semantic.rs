///! Main semantic perceiver implementation
use crate::{
    classifier::Classifier, errors::*, keywords::KeywordExtractor, language::LanguageDetector,
    models::*, summarizer::Summarizer,
};
use async_trait::async_trait;
use perceiver_structural::StructuralPerceiver;
use soulbrowser_core_types::ExecRoute;
use std::sync::Arc;

/// Semantic perceiver trait
#[async_trait]
pub trait SemanticPerceiver: Send + Sync {
    /// Extract text content from page
    async fn extract_text(
        &self,
        route: &ExecRoute,
        options: TextExtractionOptions,
    ) -> Result<ExtractedText>;

    /// Perform full semantic analysis on page
    async fn analyze(
        &self,
        route: &ExecRoute,
        options: SemanticOptions,
    ) -> Result<SemanticAnalysisResult>;

    /// Analyze already extracted text
    async fn analyze_text(
        &self,
        text: &ExtractedText,
        options: SemanticOptions,
    ) -> Result<SemanticAnalysisResult>;
}

/// Semantic perceiver implementation
pub struct SemanticPerceiverImpl {
    structural_perceiver: Arc<dyn StructuralPerceiver>,
    language_detector: LanguageDetector,
    classifier: Classifier,
    summarizer: Summarizer,
    keyword_extractor: KeywordExtractor,
}

impl SemanticPerceiverImpl {
    /// Create new semantic perceiver
    pub fn new(structural_perceiver: Arc<dyn StructuralPerceiver>) -> Self {
        Self {
            structural_perceiver,
            language_detector: LanguageDetector::new(),
            classifier: Classifier::new(),
            summarizer: Summarizer::new(),
            keyword_extractor: KeywordExtractor::new(),
        }
    }

    /// Extract text from structural perceiver DOM
    async fn extract_text_from_dom(
        &self,
        route: &ExecRoute,
        _options: &TextExtractionOptions,
    ) -> Result<ExtractedText> {
        // Get DOM snapshot from structural perceiver
        let snapshot = self
            .structural_perceiver
            .snapshot_dom_ax(route.clone())
            .await
            .map_err(|e| SemanticError::StructuralError(format!("{:?}", e)))?;

        // For MVP, extract text from JSON representation
        // In a full implementation, we would parse the DOM structure properly

        // Simple text extraction from JSON
        let mut body_text = Vec::new();
        let mut headings = Vec::new();

        // Extract text nodes (simplified approach)
        if let Some(dom_obj) = snapshot.dom_raw.as_object() {
            Self::extract_text_from_value(
                &dom_obj.get("nodes").unwrap_or(&serde_json::Value::Null),
                &mut body_text,
                &mut headings,
            );
        }

        let body = body_text.join(" ");
        let char_count = body.len();

        Ok(ExtractedText {
            body,
            title: None,       // TODO: Extract from DOM
            description: None, // TODO: Extract from meta tags
            headings,
            links: vec![], // TODO: Extract links
            char_count,
        })
    }

    /// Recursively extract text from JSON value
    fn extract_text_from_value(
        value: &serde_json::Value,
        body_text: &mut Vec<String>,
        headings: &mut Vec<String>,
    ) {
        match value {
            serde_json::Value::Object(obj) => {
                // Check for node type
                if let Some(node_type) = obj.get("nodeType") {
                    if node_type == 3 {
                        // Text node
                        if let Some(text) = obj.get("nodeValue").and_then(|v| v.as_str()) {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                body_text.push(trimmed.to_string());
                            }
                        }
                    }
                }

                // Check for heading nodes
                if let Some(node_name) = obj.get("nodeName").and_then(|v| v.as_str()) {
                    if matches!(node_name, "H1" | "H2" | "H3" | "H4" | "H5" | "H6") {
                        if let Some(text) = obj.get("nodeValue").and_then(|v| v.as_str()) {
                            headings.push(text.to_string());
                        }
                    }
                }

                // Recursively process child nodes
                for (_, v) in obj {
                    Self::extract_text_from_value(v, body_text, headings);
                }
            }
            serde_json::Value::Array(arr) => {
                for v in arr {
                    Self::extract_text_from_value(v, body_text, headings);
                }
            }
            _ => {}
        }
    }
}

#[async_trait]
impl SemanticPerceiver for SemanticPerceiverImpl {
    async fn extract_text(
        &self,
        route: &ExecRoute,
        options: TextExtractionOptions,
    ) -> Result<ExtractedText> {
        self.extract_text_from_dom(route, &options).await
    }

    async fn analyze(
        &self,
        route: &ExecRoute,
        options: SemanticOptions,
    ) -> Result<SemanticAnalysisResult> {
        // Extract text first
        let text = self
            .extract_text(route, TextExtractionOptions::default())
            .await?;

        // Analyze the extracted text
        self.analyze_text(&text, options).await
    }

    async fn analyze_text(
        &self,
        text: &ExtractedText,
        options: SemanticOptions,
    ) -> Result<SemanticAnalysisResult> {
        // Run analysis components in parallel
        let all_text = text.all_text();

        // Language detection
        let language = self.language_detector.detect(&all_text)?;

        // Content classification (run in blocking task)
        let text_clone = text.clone();
        let classifier = self.classifier.clone();
        let content_type =
            tokio::task::spawn_blocking(move || classifier.classify_content_type(&text_clone))
                .await
                .map_err(|e| SemanticError::AnalysisFailed(format!("Task join error: {}", e)))??;

        // Intent classification
        let text_clone = text.clone();
        let classifier = self.classifier.clone();
        let intent = tokio::task::spawn_blocking(move || classifier.classify_intent(&text_clone))
            .await
            .map_err(|e| SemanticError::AnalysisFailed(format!("Task join error: {}", e)))??;

        // Summarization
        let text_clone = text.clone();
        let summarizer = self.summarizer.clone();
        let summary = tokio::task::spawn_blocking(move || summarizer.summarize(&text_clone))
            .await
            .map_err(|e| SemanticError::SummarizationFailed(format!("Task join error: {}", e)))??;

        // Keyword extraction
        let keywords = if options.extract_keywords {
            let text_clone = text.clone();
            let options_clone = options.clone();
            let extractor = self.keyword_extractor.clone();
            tokio::task::spawn_blocking(move || extractor.extract(&text_clone, &options_clone))
                .await
                .map_err(|e| SemanticError::AnalysisFailed(format!("Task join error: {}", e)))??
        } else {
            std::collections::HashMap::new()
        };

        // Readability analysis
        let readability = if options.analyze_readability {
            let all_text_clone = all_text.clone();
            let summarizer = self.summarizer.clone();
            Some(
                tokio::task::spawn_blocking(move || {
                    summarizer.calculate_readability(&all_text_clone)
                })
                .await
                .map_err(|e| SemanticError::AnalysisFailed(format!("Task join error: {}", e)))?,
            )
        } else {
            None
        };

        // Entity extraction (placeholder - would need NLP library)
        let entities = if options.extract_entities {
            Vec::new() // TODO: Implement with advanced NLP
        } else {
            Vec::new()
        };

        // Sentiment analysis (placeholder - would need NLP library)
        let sentiment = if options.analyze_sentiment {
            None // TODO: Implement with advanced NLP
        } else {
            None
        };

        Ok(SemanticAnalysisResult {
            content_type,
            intent,
            language,
            summary,
            entities,
            keywords,
            sentiment,
            readability,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_analyze_text() {
        // Create mock structural perceiver
        // This test would need proper mocking infrastructure
        // For now, we test the components individually in their own modules
    }
}
