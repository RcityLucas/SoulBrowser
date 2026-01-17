//! Visual Perceiver - Screenshot capture, OCR, and visual analysis for SoulBrowser
//!
//! This crate provides visual perception capabilities including:
//! - Screenshot capture via CDP
//! - OCR text extraction (optional feature)
//! - Visual diff computation
//! - Element visual detection
//! - Visual metrics analysis

pub mod errors;
pub mod models;
pub mod screenshot;
pub mod visual;

#[cfg(feature = "ocr")]
pub mod ocr;

pub mod cache;
pub mod diff;
pub mod metrics;

// Re-exports
pub use errors::VisualError;
pub use models::*;
pub use screenshot::ScreenshotCapture;
pub use visual::{VisualPerceiver, VisualPerceiverImpl};

#[cfg(feature = "ocr")]
pub use ocr::OcrEngine;

pub use diff::VisualDiff;
pub use metrics::VisualMetrics;
