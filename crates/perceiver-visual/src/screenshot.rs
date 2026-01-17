///! Screenshot capture implementation via CDP adapter
use crate::{errors::VisualError, models::*};
use cdp_adapter::{Cdp, CdpAdapter, PageId};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

/// Screenshot capture service
pub struct ScreenshotCapture {
    cdp_adapter: Arc<CdpAdapter>,
}

impl ScreenshotCapture {
    pub fn new(cdp_adapter: Arc<CdpAdapter>) -> Self {
        Self { cdp_adapter }
    }

    /// Capture screenshot of specified page
    pub async fn capture(
        &self,
        page_id: PageId,
        options: ScreenshotOptions,
    ) -> Result<Screenshot, VisualError> {
        tracing::debug!(
            "Capturing screenshot for page {:?} with options: {:?}",
            page_id,
            options
        );

        // Use CDP adapter's screenshot method
        // Note: CDP adapter currently only supports PNG format and viewport capture
        // TODO: Add support for JPEG format and clipping when CDP adapter is enhanced
        let deadline = Duration::from_secs(30);
        let data = self
            .cdp_adapter
            .screenshot(page_id, deadline)
            .await
            .map_err(|e| VisualError::CdpError(format!("Screenshot failed: {:?}", e)))?;

        // Get image dimensions
        let (width, height) = self.get_image_dimensions(&data, ImageFormat::Png)?;

        Ok(Screenshot {
            id: Uuid::new_v4().to_string(),
            data,
            format: ImageFormat::Png, // CDP adapter always returns PNG
            width,
            height,
            timestamp: SystemTime::now(),
            page_id: page_id.0.to_string(),
            capture_mode: CaptureMode::Viewport, // CDP adapter captures viewport
            clip: None,                          // CDP adapter doesn't support clipping yet
        })
    }

    /// Get image dimensions from raw data
    fn get_image_dimensions(
        &self,
        data: &[u8],
        _format: ImageFormat,
    ) -> Result<(u32, u32), VisualError> {
        use image::io::Reader as ImageReader;
        use std::io::Cursor;

        let img = ImageReader::new(Cursor::new(data))
            .with_guessed_format()
            .map_err(|e| VisualError::ImageProcessing(format!("Format detection failed: {}", e)))?
            .decode()
            .map_err(|e| VisualError::ImageProcessing(format!("Image decode failed: {}", e)))?;

        Ok((img.width(), img.height()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_screenshot_options_default() {
        let options = ScreenshotOptions::default();
        assert_eq!(options.format, ImageFormat::Png);
        assert_eq!(options.capture_mode, CaptureMode::Viewport);
        assert!(options.use_cache);
        assert_eq!(options.cache_ttl_secs, 60);
    }

    #[test]
    fn test_bounding_box_creation() {
        let bbox = BoundingBox {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        };
        assert_eq!(bbox.x, 10.0);
        assert_eq!(bbox.width, 100.0);
    }
}
