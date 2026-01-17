///! Visual metrics extraction from screenshots
use crate::{errors::VisualError, models::*};
use image::{DynamicImage, GenericImageView, Rgba};
use std::collections::HashMap;

/// Visual metrics analyzer
pub struct VisualMetrics;

impl VisualMetrics {
    /// Analyze visual metrics from screenshot
    pub fn analyze(screenshot: &Screenshot) -> Result<VisualMetricsResult, VisualError> {
        let img = Self::decode_image(&screenshot.data)?;

        let color_palette = Self::extract_color_palette(&img, 5)?;
        let avg_contrast_ratio = Self::calculate_avg_contrast(&img)?;
        let layout_stability = Self::estimate_layout_stability(&img)?;
        let viewport_utilization = Self::calculate_viewport_utilization(&img)?;

        Ok(VisualMetricsResult {
            color_palette,
            avg_contrast_ratio,
            layout_stability,
            viewport_utilization,
        })
    }

    fn decode_image(data: &[u8]) -> Result<DynamicImage, VisualError> {
        image::load_from_memory(data)
            .map_err(|e| VisualError::ImageProcessing(format!("Failed to decode image: {}", e)))
    }

    fn extract_color_palette(
        img: &DynamicImage,
        top_n: usize,
    ) -> Result<Vec<(u8, u8, u8)>, VisualError> {
        let (width, height) = img.dimensions();
        let mut color_counts: HashMap<(u8, u8, u8), u32> = HashMap::new();

        // Sample every Nth pixel to improve performance
        let sample_rate = 5;

        for y in (0..height).step_by(sample_rate) {
            for x in (0..width).step_by(sample_rate) {
                let pixel = img.get_pixel(x, y);
                let color = (pixel[0], pixel[1], pixel[2]);
                *color_counts.entry(color).or_insert(0) += 1;
            }
        }

        // Sort by frequency and return top N
        let mut colors: Vec<_> = color_counts.into_iter().collect();
        colors.sort_by(|a, b| b.1.cmp(&a.1));

        Ok(colors
            .into_iter()
            .take(top_n)
            .map(|(color, _)| color)
            .collect())
    }

    fn calculate_avg_contrast(img: &DynamicImage) -> Result<f64, VisualError> {
        let (width, height) = img.dimensions();
        let mut total_contrast = 0.0;
        let mut count = 0u64;

        let sample_rate = 10;

        for y in (0..height.saturating_sub(1)).step_by(sample_rate) {
            for x in (0..width.saturating_sub(1)).step_by(sample_rate) {
                let p1 = img.get_pixel(x, y);
                let p2 = img.get_pixel(x + 1, y);
                let p3 = img.get_pixel(x, y + 1);

                total_contrast += Self::contrast_ratio(&p1, &p2);
                total_contrast += Self::contrast_ratio(&p1, &p3);
                count += 2;
            }
        }

        if count == 0 {
            return Ok(0.0);
        }

        Ok(total_contrast / count as f64)
    }

    fn contrast_ratio(p1: &Rgba<u8>, p2: &Rgba<u8>) -> f64 {
        let l1 = Self::relative_luminance(p1);
        let l2 = Self::relative_luminance(p2);

        let lighter = l1.max(l2);
        let darker = l1.min(l2);

        (lighter + 0.05) / (darker + 0.05)
    }

    fn relative_luminance(pixel: &Rgba<u8>) -> f64 {
        let r = pixel[0] as f64 / 255.0;
        let g = pixel[1] as f64 / 255.0;
        let b = pixel[2] as f64 / 255.0;

        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    fn estimate_layout_stability(_img: &DynamicImage) -> Result<f64, VisualError> {
        // Simplified layout stability estimation
        // In real implementation, would compare with previous screenshot
        // For now, return a placeholder value
        Ok(0.95)
    }

    fn calculate_viewport_utilization(img: &DynamicImage) -> Result<f64, VisualError> {
        let (width, height) = img.dimensions();
        let mut non_white_pixels = 0u64;

        let sample_rate = 5;

        for y in (0..height).step_by(sample_rate) {
            for x in (0..width).step_by(sample_rate) {
                let pixel = img.get_pixel(x, y);
                // Check if pixel is not close to white
                if pixel[0] < 250 || pixel[1] < 250 || pixel[2] < 250 {
                    non_white_pixels += 1;
                }
            }
        }

        let sampled_total = ((width / sample_rate as u32) * (height / sample_rate as u32)) as f64;
        Ok((non_white_pixels as f64 / sampled_total).max(0.0).min(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageBuffer;
    use std::time::SystemTime;

    fn create_test_screenshot(width: u32, height: u32, color: Rgba<u8>) -> Screenshot {
        let img = ImageBuffer::from_pixel(width, height, color);
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();

        Screenshot {
            id: "test".to_string(),
            data: buf,
            format: ImageFormat::Png,
            width,
            height,
            timestamp: SystemTime::now(),
            page_id: "page-1".to_string(),
            capture_mode: CaptureMode::Viewport,
            clip: None,
        }
    }

    #[test]
    fn test_color_palette_extraction() {
        let screenshot = create_test_screenshot(100, 100, Rgba([255, 0, 0, 255]));
        let metrics = VisualMetrics::analyze(&screenshot).unwrap();

        assert!(!metrics.color_palette.is_empty());
        assert_eq!(metrics.color_palette[0], (255, 0, 0));
    }

    #[test]
    fn test_viewport_utilization() {
        let screenshot = create_test_screenshot(100, 100, Rgba([255, 0, 0, 255]));
        let metrics = VisualMetrics::analyze(&screenshot).unwrap();

        assert!(metrics.viewport_utilization > 0.0);
        assert!(metrics.viewport_utilization <= 1.0);
    }
}
