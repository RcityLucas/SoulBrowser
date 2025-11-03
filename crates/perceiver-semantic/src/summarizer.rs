///! Content summarization module
use crate::{errors::*, models::*};
use unicode_segmentation::UnicodeSegmentation;

/// Text summarizer
#[derive(Clone)]
pub struct Summarizer;

impl Summarizer {
    /// Create new summarizer
    pub fn new() -> Self {
        Self
    }

    /// Generate content summary
    pub fn summarize(&self, text: &ExtractedText) -> Result<ContentSummary> {
        let all_text = text.all_text();

        // Count words
        let word_count = self.count_words(&all_text);

        // Extract key sentences
        let sentences = self.extract_sentences(&all_text);

        // Generate short summary (first 1-2 sentences)
        let short = if !sentences.is_empty() {
            let first_two: Vec<_> = sentences.iter().take(2).cloned().collect();
            first_two.join(" ")
        } else {
            String::new()
        };

        // Generate medium summary (first paragraph or 3-5 sentences)
        let medium = if sentences.len() > 2 {
            let first_five: Vec<_> = sentences.iter().take(5).cloned().collect();
            Some(first_five.join(" "))
        } else {
            None
        };

        // Extract key points from headings and important sentences
        let key_points = self.extract_key_points(text, &sentences);

        Ok(ContentSummary {
            short,
            medium,
            key_points,
            word_count,
        })
    }

    /// Count words in text
    fn count_words(&self, text: &str) -> usize {
        text.unicode_words().count()
    }

    /// Extract sentences from text
    fn extract_sentences(&self, text: &str) -> Vec<String> {
        text.split(|c| c == '.' || c == '!' || c == '?')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.len() > 10) // Filter out very short fragments
            .take(10) // Limit to first 10 sentences
            .collect()
    }

    /// Extract key points from headings and sentences
    fn extract_key_points(&self, text: &ExtractedText, sentences: &[String]) -> Vec<String> {
        let mut key_points = Vec::new();

        // Add headings as key points
        for heading in &text.headings {
            if !heading.is_empty() {
                key_points.push(heading.clone());
            }
        }

        // If we don't have enough headings, add important sentences
        if key_points.len() < 3 && !sentences.is_empty() {
            // Add first sentence if not already included
            if let Some(first) = sentences.first() {
                if !key_points.contains(first) {
                    key_points.push(first.clone());
                }
            }

            // Add sentences with important keywords
            for sentence in sentences.iter().skip(1) {
                if key_points.len() >= 5 {
                    break;
                }

                // Check for important keywords
                let lower = sentence.to_lowercase();
                if lower.contains("important")
                    || lower.contains("key")
                    || lower.contains("main")
                    || lower.contains("primary")
                {
                    key_points.push(sentence.clone());
                }
            }
        }

        // Limit to 5 key points
        key_points.truncate(5);
        key_points
    }

    /// Calculate readability score (Flesch Reading Ease)
    pub fn calculate_readability(&self, text: &str) -> f64 {
        let sentences = self.extract_sentences(text);
        let sentence_count = sentences.len() as f64;

        if sentence_count == 0.0 {
            return 0.0;
        }

        let word_count = self.count_words(text) as f64;
        let syllable_count = self.estimate_syllables(text) as f64;

        // Flesch Reading Ease formula
        // Score = 206.835 - 1.015 * (words/sentences) - 84.6 * (syllables/words)
        let avg_sentence_length = word_count / sentence_count;
        let avg_syllables_per_word = syllable_count / word_count.max(1.0);

        let score = 206.835 - 1.015 * avg_sentence_length - 84.6 * avg_syllables_per_word;

        // Clamp to 0-100 range
        score.max(0.0).min(100.0)
    }

    /// Estimate syllable count (simplified algorithm)
    fn estimate_syllables(&self, text: &str) -> usize {
        let words = text.unicode_words();
        let mut total_syllables = 0;

        for word in words {
            let syllables = self.count_syllables_in_word(word);
            total_syllables += syllables;
        }

        total_syllables
    }

    /// Count syllables in a single word (simplified)
    fn count_syllables_in_word(&self, word: &str) -> usize {
        let vowels = ['a', 'e', 'i', 'o', 'u', 'y'];
        let word_lower = word.to_lowercase();
        let chars: Vec<char> = word_lower.chars().collect();

        let mut syllable_count = 0;
        let mut previous_was_vowel = false;

        for ch in chars.iter() {
            if vowels.contains(ch) {
                if !previous_was_vowel {
                    syllable_count += 1;
                }
                previous_was_vowel = true;
            } else {
                previous_was_vowel = false;
            }
        }

        // Handle silent 'e'
        if word_lower.ends_with('e') && syllable_count > 1 {
            syllable_count -= 1;
        }

        // Every word has at least one syllable
        syllable_count.max(1)
    }
}

impl Default for Summarizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_word_count() {
        let summarizer = Summarizer::new();
        let count = summarizer.count_words("This is a test sentence.");
        assert_eq!(count, 5);
    }

    #[test]
    fn test_extract_sentences() {
        let summarizer = Summarizer::new();
        let sentences = summarizer.extract_sentences("First sentence. Second sentence! Third?");

        // "Third" is too short (< 10 chars) so it gets filtered out
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0], "First sentence");
        assert_eq!(sentences[1], "Second sentence");
    }

    #[test]
    fn test_summarize() {
        let summarizer = Summarizer::new();
        let text = ExtractedText {
            body: "This is the first sentence. This is the second. This is the third.".to_string(),
            title: Some("Test Title".to_string()),
            description: None,
            headings: vec!["Heading One".to_string(), "Heading Two".to_string()],
            links: vec![],
            char_count: 100,
        };

        let summary = summarizer.summarize(&text).unwrap();

        assert!(!summary.short.is_empty());
        assert!(summary.word_count > 0);
        assert!(!summary.key_points.is_empty());
    }

    #[test]
    fn test_syllable_count() {
        let summarizer = Summarizer::new();

        assert_eq!(summarizer.count_syllables_in_word("cat"), 1);
        assert_eq!(summarizer.count_syllables_in_word("happy"), 2);
        assert_eq!(summarizer.count_syllables_in_word("beautiful"), 3);
    }

    #[test]
    fn test_readability() {
        let summarizer = Summarizer::new();
        let text = "The cat sat on the mat. It was a nice day.";

        let score = summarizer.calculate_readability(text);

        // Simple text should have high readability score
        assert!(score > 50.0);
        assert!(score <= 100.0);
    }
}
