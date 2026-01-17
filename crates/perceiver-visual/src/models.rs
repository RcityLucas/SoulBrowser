///! Data models for visual perception
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Screenshot captured from a web page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Screenshot {
    /// Unique identifier for the screenshot
    pub id: String,

    /// Raw image data (PNG or JPEG)
    pub data: Vec<u8>,

    /// Image format
    pub format: ImageFormat,

    /// Image dimensions
    pub width: u32,
    pub height: u32,

    /// Capture timestamp
    pub timestamp: SystemTime,

    /// Page ID this screenshot belongs to
    pub page_id: String,

    /// Viewport or full page capture
    pub capture_mode: CaptureMode,

    /// Optional clipping region
    pub clip: Option<BoundingBox>,
}

/// Image format for screenshots
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

/// Screenshot capture mode
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CaptureMode {
    /// Capture visible viewport only
    Viewport,

    /// Capture entire scrollable page
    FullPage,
}

/// Bounding box for regions
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Options for screenshot capture
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotOptions {
    /// Image format
    pub format: ImageFormat,

    /// JPEG quality (0-100, only for JPEG format)
    pub quality: Option<u8>,

    /// Capture mode
    pub capture_mode: CaptureMode,

    /// Optional clipping region
    pub clip: Option<BoundingBox>,

    /// Use cached screenshot if available within TTL
    pub use_cache: bool,

    /// Cache TTL in seconds
    pub cache_ttl_secs: u64,
}

impl Default for ScreenshotOptions {
    fn default() -> Self {
        Self {
            format: ImageFormat::Png,
            quality: None,
            capture_mode: CaptureMode::Viewport,
            clip: None,
            use_cache: true,
            cache_ttl_secs: 60,
        }
    }
}

/// OCR options (optional feature)
#[cfg(feature = "ocr")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrOptions {
    /// Language code (e.g., "eng", "chi_sim")
    pub language: String,

    /// Page segmentation mode
    pub psm: PageSegMode,

    /// Character whitelist (None for no restriction)
    pub whitelist: Option<String>,

    /// Minimum confidence threshold (0.0-1.0)
    pub min_confidence: f64,
}

#[cfg(feature = "ocr")]
impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            language: "eng".to_string(),
            psm: PageSegMode::Auto,
            whitelist: None,
            min_confidence: 0.0,
        }
    }
}

/// Page segmentation mode for OCR
#[cfg(feature = "ocr")]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PageSegMode {
    Auto,
    SingleBlock,
    SingleLine,
    SingleWord,
    SingleChar,
}

/// OCR result
#[cfg(feature = "ocr")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrResult {
    /// Extracted text
    pub text: String,

    /// Average confidence (0.0-1.0)
    pub confidence: f64,

    /// Text blocks with positions
    pub blocks: Vec<TextBlock>,

    /// Processing time in milliseconds
    pub processing_time_ms: u64,
}

#[cfg(feature = "ocr")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextBlock {
    pub text: String,
    pub confidence: f64,
    pub bounds: BoundingBox,
}

/// Visual diff options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffOptions {
    /// Pixel difference threshold (0.0-1.0)
    pub pixel_threshold: f64,

    /// Generate diff image highlighting changes
    pub generate_diff_image: bool,

    /// Highlight color for differences (RGB)
    pub highlight_color: Option<(u8, u8, u8)>,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            pixel_threshold: 0.01, // 1% difference
            generate_diff_image: false,
            highlight_color: Some((255, 0, 0)), // Red
        }
    }
}

/// Visual diff result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualDiffResult {
    /// Percentage of pixels that differ (0.0-100.0)
    pub pixel_diff_percent: f64,

    /// Structural similarity index (0.0-1.0, 1.0 = identical)
    pub structural_similarity: f64,

    /// Diff image data (if requested)
    pub diff_image: Option<Vec<u8>>,

    /// Regions with significant changes
    pub changed_regions: Vec<BoundingBox>,

    /// Are images significantly different?
    pub is_different: bool,
}

/// Visual element detected in screenshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualElement {
    /// Element type (button, link, input, etc.)
    pub element_type: ElementType,

    /// Bounding box
    pub bounds: BoundingBox,

    /// Confidence score (0.0-1.0)
    pub confidence: f64,

    /// Visual properties
    pub properties: VisualProperties,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ElementType {
    Button,
    Link,
    Input,
    Image,
    Text,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualProperties {
    /// Dominant color (RGB)
    pub dominant_color: Option<(u8, u8, u8)>,

    /// Is visually prominent
    pub is_prominent: bool,

    /// Contrast ratio with background
    pub contrast_ratio: Option<f64>,
}

/// Visual metrics for a screenshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualMetricsResult {
    /// Color palette (top N colors)
    pub color_palette: Vec<(u8, u8, u8)>,

    /// Average contrast ratio
    pub avg_contrast_ratio: f64,

    /// Layout stability score (0.0-1.0)
    pub layout_stability: f64,

    /// Viewport utilization (0.0-1.0)
    pub viewport_utilization: f64,
}
