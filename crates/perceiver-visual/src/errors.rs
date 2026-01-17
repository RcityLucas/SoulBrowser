///! Error types for visual perceiver operations
use std::fmt;

#[derive(Debug)]
pub enum VisualError {
    /// Screenshot capture failed
    CaptureFailed(String),

    /// Image processing error
    ImageProcessing(String),

    /// OCR operation failed
    OcrFailed(String),

    /// Visual diff computation failed
    DiffFailed(String),

    /// Cache error
    CacheError(String),

    /// CDP adapter error
    CdpError(String),

    /// Invalid input parameters
    InvalidInput(String),

    /// IO error
    Io(std::io::Error),
}

impl fmt::Display for VisualError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CaptureFailed(msg) => write!(f, "Screenshot capture failed: {}", msg),
            Self::ImageProcessing(msg) => write!(f, "Image processing error: {}", msg),
            Self::OcrFailed(msg) => write!(f, "OCR operation failed: {}", msg),
            Self::DiffFailed(msg) => write!(f, "Visual diff failed: {}", msg),
            Self::CacheError(msg) => write!(f, "Cache error: {}", msg),
            Self::CdpError(msg) => write!(f, "CDP adapter error: {}", msg),
            Self::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            Self::Io(err) => write!(f, "IO error: {}", err),
        }
    }
}

impl std::error::Error for VisualError {}

impl From<std::io::Error> for VisualError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<image::ImageError> for VisualError {
    fn from(err: image::ImageError) -> Self {
        Self::ImageProcessing(err.to_string())
    }
}

#[cfg(feature = "ocr")]
impl From<tesseract::TessErr> for VisualError {
    fn from(err: tesseract::TessErr) -> Self {
        Self::OcrFailed(err.to_string())
    }
}
