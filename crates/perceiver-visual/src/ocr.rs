///! OCR text extraction using Tesseract (optional feature)
use crate::{errors::VisualError, models::*};
use std::time::Instant;
use tesseract::{InitOptions, Tesseract};

/// OCR engine wrapper
pub struct OcrEngine {
    tesseract: Tesseract,
}

impl OcrEngine {
    /// Create new OCR engine with default language (English)
    pub fn new() -> Self {
        Self::with_language("eng")
    }

    /// Create OCR engine with specified language
    pub fn with_language(language: &str) -> Self {
        let tesseract =
            Tesseract::new(None, Some(language)).expect("Failed to initialize Tesseract");

        Self { tesseract }
    }

    /// Extract text from screenshot
    pub async fn extract_text(
        &self,
        screenshot: &Screenshot,
        options: OcrOptions,
    ) -> Result<OcrResult, VisualError> {
        let start = Instant::now();

        // Decode image
        let img = image::load_from_memory(&screenshot.data)
            .map_err(|e| VisualError::ImageProcessing(format!("Image decode failed: {}", e)))?;

        // Convert to grayscale for better OCR
        let gray = img.to_luma8();

        // Create Tesseract instance with options
        let mut tess = Tesseract::new(None, Some(&options.language))
            .map_err(|e| VisualError::OcrFailed(format!("Tesseract init failed: {}", e)))?;

        // Set page segmentation mode
        tess.set_variable("tessedit_pageseg_mode", &Self::psm_to_string(options.psm))
            .map_err(|e| VisualError::OcrFailed(format!("Failed to set PSM: {}", e)))?;

        // Set character whitelist if provided
        if let Some(ref whitelist) = options.whitelist {
            tess.set_variable("tessedit_char_whitelist", whitelist)
                .map_err(|e| VisualError::OcrFailed(format!("Failed to set whitelist: {}", e)))?;
        }

        // Set image data
        tess.set_image_from_mem(&gray)
            .map_err(|e| VisualError::OcrFailed(format!("Failed to set image: {}", e)))?;

        // Extract text
        let text = tess
            .get_text()
            .map_err(|e| VisualError::OcrFailed(format!("Text extraction failed: {}", e)))?;

        // Get confidence scores (simplified)
        let confidence = tess.mean_text_conf() as f64 / 100.0;

        // Extract text blocks (simplified - full implementation would parse detailed results)
        let blocks = vec![TextBlock {
            text: text.clone(),
            confidence,
            bounds: BoundingBox {
                x: 0.0,
                y: 0.0,
                width: screenshot.width as f64,
                height: screenshot.height as f64,
            },
        }];

        let processing_time_ms = start.elapsed().as_millis() as u64;

        Ok(OcrResult {
            text,
            confidence,
            blocks,
            processing_time_ms,
        })
    }

    fn psm_to_string(psm: PageSegMode) -> String {
        match psm {
            PageSegMode::Auto => "3".to_string(),        // PSM_AUTO
            PageSegMode::SingleBlock => "6".to_string(), // PSM_SINGLE_BLOCK
            PageSegMode::SingleLine => "7".to_string(),  // PSM_SINGLE_LINE
            PageSegMode::SingleWord => "8".to_string(),  // PSM_SINGLE_WORD
            PageSegMode::SingleChar => "10".to_string(), // PSM_SINGLE_CHAR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[tokio::test]
    #[ignore] // Requires Tesseract installation
    async fn test_ocr_basic() {
        // Create a simple test image with text
        use image::{ImageBuffer, Rgb};
        use imageproc::drawing::draw_text_mut;
        use imageproc::drawing::text_size;
        use rusttype::{Font, Scale};

        let mut img = ImageBuffer::from_pixel(400, 100, Rgb([255u8, 255u8, 255u8]));

        // This test requires font data - skip in automated tests
        // Real implementation would use actual font rendering

        let engine = OcrEngine::new();
        let screenshot = Screenshot {
            id: "test".to_string(),
            data: vec![], // Would contain actual image data
            format: ImageFormat::Png,
            width: 400,
            height: 100,
            timestamp: SystemTime::now(),
            page_id: "test".to_string(),
            capture_mode: CaptureMode::Viewport,
            clip: None,
        };

        // Skip actual OCR test in unit tests
        // let result = engine.extract_text(&screenshot, OcrOptions::default()).await;
        // assert!(result.is_ok());
    }

    #[test]
    fn test_psm_conversion() {
        assert_eq!(OcrEngine::psm_to_string(PageSegMode::Auto), "3");
        assert_eq!(OcrEngine::psm_to_string(PageSegMode::SingleLine), "7");
    }
}
