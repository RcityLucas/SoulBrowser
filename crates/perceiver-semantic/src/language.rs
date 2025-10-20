///! Language detection module

use crate::{errors::*, models::*};
use whatlang::{detect, Lang};

/// Language detector
pub struct LanguageDetector;

impl LanguageDetector {
    /// Create new language detector
    pub fn new() -> Self {
        Self
    }

    /// Detect language from text
    pub fn detect(&self, text: &str) -> Result<LanguageInfo> {
        if text.trim().is_empty() {
            return Err(SemanticError::InvalidInput(
                "Cannot detect language from empty text".to_string(),
            ));
        }

        // Use whatlang for detection
        let info = detect(text).ok_or_else(|| {
            SemanticError::LanguageDetectionFailed(
                "Could not detect language from text".to_string(),
            )
        })?;

        Ok(LanguageInfo {
            code: Self::lang_to_code(info.lang()),
            name: Self::lang_to_name(info.lang()),
            confidence: info.confidence(),
        })
    }

    /// Convert whatlang Lang to ISO 639-1 code
    fn lang_to_code(lang: Lang) -> String {
        match lang {
            Lang::Eng => "en",
            Lang::Cmn => "zh",
            Lang::Spa => "es",
            Lang::Fra => "fr",
            Lang::Deu => "de",
            Lang::Rus => "ru",
            Lang::Jpn => "ja",
            Lang::Por => "pt",
            Lang::Ita => "it",
            Lang::Kor => "ko",
            Lang::Ara => "ar",
            Lang::Hin => "hi",
            Lang::Ben => "bn",
            Lang::Vie => "vi",
            Lang::Tha => "th",
            _ => "unknown",
        }
        .to_string()
    }

    /// Convert whatlang Lang to human-readable name
    fn lang_to_name(lang: Lang) -> String {
        match lang {
            Lang::Eng => "English",
            Lang::Cmn => "Chinese",
            Lang::Spa => "Spanish",
            Lang::Fra => "French",
            Lang::Deu => "German",
            Lang::Rus => "Russian",
            Lang::Jpn => "Japanese",
            Lang::Por => "Portuguese",
            Lang::Ita => "Italian",
            Lang::Kor => "Korean",
            Lang::Ara => "Arabic",
            Lang::Hin => "Hindi",
            Lang::Ben => "Bengali",
            Lang::Vie => "Vietnamese",
            Lang::Tha => "Thai",
            _ => "Unknown",
        }
        .to_string()
    }
}

impl Default for LanguageDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_english() {
        let detector = LanguageDetector::new();
        let result = detector
            .detect("This is a test sentence in English.")
            .unwrap();

        assert_eq!(result.code, "en");
        assert_eq!(result.name, "English");
        // Confidence can vary, just check it's reasonable
        assert!(result.confidence > 0.5);
    }

    #[test]
    fn test_detect_chinese() {
        let detector = LanguageDetector::new();
        let result = detector.detect("这是一个中文测试句子。").unwrap();

        assert_eq!(result.code, "zh");
        assert_eq!(result.name, "Chinese");
        assert!(result.confidence > 0.8);
    }

    #[test]
    fn test_empty_text_error() {
        let detector = LanguageDetector::new();
        let result = detector.detect("");

        assert!(result.is_err());
    }
}
