///! Keyword extraction module

use crate::{errors::*, models::*};
use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;

/// Keyword extractor
#[derive(Clone)]
pub struct KeywordExtractor {
    stop_words: Vec<String>,
}

impl KeywordExtractor {
    /// Create new keyword extractor
    pub fn new() -> Self {
        Self {
            stop_words: Self::get_common_stop_words(),
        }
    }

    /// Extract keywords from text
    pub fn extract(&self, text: &ExtractedText, options: &SemanticOptions) -> Result<HashMap<String, f64>> {
        let all_text = text.all_text();
        let words = self.tokenize(&all_text);

        // Filter out stop words and short words
        let filtered_words: Vec<String> = words
            .into_iter()
            .filter(|w| {
                w.len() > 2 && !self.stop_words.contains(&w.to_lowercase())
            })
            .collect();

        // Calculate term frequency
        let mut term_freq: HashMap<String, usize> = HashMap::new();
        for word in &filtered_words {
            *term_freq.entry(word.to_lowercase()).or_insert(0) += 1;
        }

        // Calculate relevance scores (normalized TF)
        let max_freq = term_freq.values().max().copied().unwrap_or(1) as f64;
        let mut keywords: HashMap<String, f64> = HashMap::new();

        for (term, freq) in term_freq {
            let score = freq as f64 / max_freq;

            // Apply keyword score threshold
            if score >= options.min_keyword_score {
                keywords.insert(term, score);
            }
        }

        // Boost keywords that appear in headings or title
        if let Some(ref title) = text.title {
            for word in self.tokenize(title) {
                let word_lower = word.to_lowercase();
                if let Some(score) = keywords.get_mut(&word_lower) {
                    *score *= 1.5; // Boost title words
                }
            }
        }

        for heading in &text.headings {
            for word in self.tokenize(heading) {
                let word_lower = word.to_lowercase();
                if let Some(score) = keywords.get_mut(&word_lower) {
                    *score *= 1.3; // Boost heading words
                }
            }
        }

        // Sort by score and take top N
        let mut sorted_keywords: Vec<_> = keywords.into_iter().collect();
        sorted_keywords.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        sorted_keywords.truncate(options.max_keywords);

        Ok(sorted_keywords.into_iter().collect())
    }

    /// Tokenize text into words
    fn tokenize(&self, text: &str) -> Vec<String> {
        text.unicode_words()
            .map(|w| w.to_string())
            .collect()
    }

    /// Get common English stop words
    fn get_common_stop_words() -> Vec<String> {
        vec![
            "a", "an", "and", "are", "as", "at", "be", "by", "for", "from",
            "has", "he", "in", "is", "it", "its", "of", "on", "that", "the",
            "to", "was", "will", "with", "the", "this", "but", "they", "have",
            "had", "what", "when", "where", "who", "which", "why", "how",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }
}

impl Default for KeywordExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let extractor = KeywordExtractor::new();
        let words = extractor.tokenize("Hello, world! This is a test.");

        assert_eq!(words.len(), 6);
        assert!(words.contains(&"Hello".to_string()));
        assert!(words.contains(&"world".to_string()));
    }

    #[test]
    fn test_extract_keywords() {
        let extractor = KeywordExtractor::new();
        let text = ExtractedText {
            body: "Rust programming language. Rust is great for systems programming. Programming in Rust.".to_string(),
            title: Some("Rust Programming".to_string()),
            description: None,
            headings: vec![],
            links: vec![],
            char_count: 100,
        };

        let options = SemanticOptions::default();
        let keywords = extractor.extract(&text, &options).unwrap();

        // "rust" and "programming" should be top keywords
        assert!(keywords.contains_key("rust"));
        assert!(keywords.contains_key("programming"));

        // Should not contain stop words
        assert!(!keywords.contains_key("is"));
        assert!(!keywords.contains_key("for"));
    }

    #[test]
    fn test_title_boost() {
        let extractor = KeywordExtractor::new();
        let text = ExtractedText {
            body: "test word another word test".to_string(),
            title: Some("important test".to_string()),
            description: None,
            headings: vec![],
            links: vec![],
            char_count: 50,
        };

        let options = SemanticOptions::default();
        let keywords = extractor.extract(&text, &options).unwrap();

        // "test" appears in title and body, should have highest score
        let test_score = keywords.get("test").unwrap();
        let word_score = keywords.get("word").unwrap();

        assert!(test_score > word_score);
    }
}
