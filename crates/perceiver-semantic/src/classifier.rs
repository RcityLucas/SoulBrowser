///! Content type and intent classification

use crate::{errors::*, models::*};
use regex::Regex;

/// Content classifier
#[derive(Clone)]
pub struct Classifier {
    // Compiled regexes for efficient matching
    product_patterns: Vec<Regex>,
    form_patterns: Vec<Regex>,
    article_patterns: Vec<Regex>,
    search_patterns: Vec<Regex>,
}

impl Classifier {
    /// Create new classifier
    pub fn new() -> Self {
        Self {
            product_patterns: vec![
                Regex::new(r"(?i)\b(price|buy|cart|checkout|product|purchase)\b").unwrap(),
                Regex::new(r"(?i)\$\d+|\d+\.\d{2}").unwrap(),
                Regex::new(r"(?i)\b(add to cart|buy now|order)\b").unwrap(),
            ],
            form_patterns: vec![
                Regex::new(r"(?i)\b(sign up|log ?in|register|subscribe|submit)\b").unwrap(),
                Regex::new(r"(?i)\b(email|password|username)\b").unwrap(),
                Regex::new(r"(?i)\b(form|input|field)\b").unwrap(),
            ],
            article_patterns: vec![
                Regex::new(r"(?i)\b(article|post|blog|news|story)\b").unwrap(),
                Regex::new(r"(?i)\b(author|published|updated)\b").unwrap(),
                Regex::new(r"(?i)\b(read more|continue reading)\b").unwrap(),
            ],
            search_patterns: vec![
                Regex::new(r"(?i)\b(search|results|found|showing)\b").unwrap(),
                Regex::new(r"(?i)\b(filter|sort|page \d+)\b").unwrap(),
            ],
        }
    }

    /// Classify content type based on text and structure
    pub fn classify_content_type(&self, text: &ExtractedText) -> Result<ContentType> {
        let all_text = text.all_text().to_lowercase();

        // Score each content type
        let mut scores = std::collections::HashMap::new();

        // Product page detection
        let product_score = self.score_patterns(&all_text, &self.product_patterns);
        scores.insert(ContentType::Product, product_score);

        // Form page detection
        let form_score = self.score_patterns(&all_text, &self.form_patterns);
        scores.insert(ContentType::Form, form_score);

        // Article detection
        let article_score = self.score_patterns(&all_text, &self.article_patterns);
        scores.insert(ContentType::Article, article_score);

        // Search results detection
        let search_score = self.score_patterns(&all_text, &self.search_patterns);
        scores.insert(ContentType::Search, search_score);

        // Error page detection
        if all_text.contains("404") || all_text.contains("error") {
            scores.insert(ContentType::Error, 3.0);
        }

        // Documentation detection
        if all_text.contains("documentation")
            || all_text.contains("api")
            || all_text.contains("reference")
        {
            scores.insert(ContentType::Documentation, 2.0);
        }

        // Find highest scoring type
        let content_type = scores
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(ct, _)| ct.clone())
            .unwrap_or(ContentType::Unknown);

        Ok(content_type)
    }

    /// Classify page intent
    pub fn classify_intent(&self, text: &ExtractedText) -> Result<PageIntent> {
        let all_text = text.all_text().to_lowercase();

        // Transactional indicators
        let transactional_keywords = [
            "buy",
            "purchase",
            "order",
            "checkout",
            "sign up",
            "subscribe",
            "register",
            "download",
        ];
        let transactional_score = transactional_keywords
            .iter()
            .filter(|k| all_text.contains(*k))
            .count();

        // Navigational indicators
        let navigational_keywords = [
            "menu",
            "sitemap",
            "directory",
            "categories",
            "browse",
            "navigation",
        ];
        let navigational_score = navigational_keywords
            .iter()
            .filter(|k| all_text.contains(*k))
            .count();

        // Interactive indicators
        let interactive_keywords = [
            "calculator",
            "tool",
            "converter",
            "generator",
            "simulator",
            "game",
        ];
        let interactive_score = interactive_keywords
            .iter()
            .filter(|k| all_text.contains(*k))
            .count();

        // Determine intent based on scores
        let intent = if transactional_score > navigational_score
            && transactional_score > interactive_score
        {
            PageIntent::Transactional
        } else if navigational_score > transactional_score
            && navigational_score > interactive_score
        {
            PageIntent::Navigational
        } else if interactive_score > 0 {
            PageIntent::Interactive
        } else if text.body.len() > 500 {
            // Long content is likely informational
            PageIntent::Informational
        } else {
            PageIntent::Unknown
        };

        Ok(intent)
    }

    /// Score text against a set of patterns
    fn score_patterns(&self, text: &str, patterns: &[Regex]) -> f64 {
        patterns
            .iter()
            .map(|p| {
                let matches = p.find_iter(text).count();
                matches as f64
            })
            .sum()
    }
}

impl Default for Classifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_text(body: &str) -> ExtractedText {
        ExtractedText {
            body: body.to_string(),
            title: None,
            description: None,
            headings: vec![],
            links: vec![],
            char_count: body.len(),
        }
    }

    #[test]
    fn test_classify_product_page() {
        let classifier = Classifier::new();
        let text = create_test_text("Buy now for $19.99! Add to cart. Product details and price.");

        let content_type = classifier.classify_content_type(&text).unwrap();
        assert_eq!(content_type, ContentType::Product);
    }

    #[test]
    fn test_classify_form_page() {
        let classifier = Classifier::new();
        let text = create_test_text("Sign up with your email and password. Register now!");

        let content_type = classifier.classify_content_type(&text).unwrap();
        assert_eq!(content_type, ContentType::Form);
    }

    #[test]
    fn test_classify_transactional_intent() {
        let classifier = Classifier::new();
        let text = create_test_text("Buy now and checkout securely!");

        let intent = classifier.classify_intent(&text).unwrap();
        assert_eq!(intent, PageIntent::Transactional);
    }

    #[test]
    fn test_classify_informational_intent() {
        let classifier = Classifier::new();
        let long_text = "a".repeat(600); // Long content
        let text = create_test_text(&long_text);

        let intent = classifier.classify_intent(&text).unwrap();
        assert_eq!(intent, PageIntent::Informational);
    }
}
