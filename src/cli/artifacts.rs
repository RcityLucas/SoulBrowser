use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as Base64, Engine as _};
use clap::{Args, ValueEnum};
use font8x8::{UnicodeFonts, BASIC_FONTS};
use image::codecs::gif::{GifEncoder, Repeat};
use image::imageops::FilterType;
use image::{Delay, Frame, GenericImageView, Rgba, RgbaImage};
use serde_json::{json, Value};
use tokio::fs;
use tracing::warn;

use super::run_bundle::load_run_bundle;
use crate::cli::constants::DEFAULT_LARGE_THRESHOLD;

#[derive(Args, Clone, Debug)]
pub struct ArtifactsArgs {
    /// Path to a saved run bundle (JSON produced by --save-run)
    #[arg(long, value_name = "FILE")]
    pub input: PathBuf,

    /// Output format for printing the manifest
    #[arg(long, value_enum, default_value = "json")]
    pub format: ArtifactFormat,

    /// Filter by step identifier
    #[arg(long)]
    pub step_id: Option<String>,

    /// Filter by dispatch label (e.g. "action" or validation name)
    #[arg(long)]
    pub dispatch: Option<String>,

    /// Filter by artifact label
    #[arg(long)]
    pub label: Option<String>,

    /// Directory to extract matching artifacts as files (base64 decoded)
    #[arg(long, value_name = "DIR")]
    pub extract: Option<PathBuf>,

    /// Path to write a summary (JSON) of matching artifacts
    #[arg(long, value_name = "FILE")]
    pub summary_path: Option<PathBuf>,

    /// Threshold in bytes for highlighting large artifacts
    #[arg(long, value_name = "BYTES", default_value_t = DEFAULT_LARGE_THRESHOLD)]
    pub large_threshold: u64,

    /// Write a BrowserUse-style animated GIF timeline built from matching image artifacts
    #[arg(long, value_name = "FILE")]
    pub gif: Option<PathBuf>,

    /// Frame delay for the generated GIF timeline (milliseconds)
    #[arg(long, value_name = "MS", default_value_t = 350)]
    pub gif_frame_delay: u32,

    /// Maximum number of frames to include in the GIF (0 = unlimited)
    #[arg(long, value_name = "COUNT", default_value_t = 150)]
    pub gif_max_frames: usize,
}

#[derive(Clone, ValueEnum, Debug)]
pub enum ArtifactFormat {
    Json,
    Yaml,
    Human,
}

pub async fn cmd_artifacts(args: ArtifactsArgs) -> Result<()> {
    let bundle = load_run_bundle(&args.input).await?;

    let artifacts_value = bundle
        .get("artifacts")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));

    let filtered = filter_artifacts(&artifacts_value, &args);
    let artifacts_array = Value::Array(filtered.clone());

    if let Some(dir) = &args.extract {
        extract_artifacts(dir, &filtered).await?;
    }

    let summary = build_artifact_summary(&filtered, args.large_threshold);

    if let Some(path) = &args.summary_path {
        save_summary(path, &summary).await?;
    }

    if let Some(path) = &args.gif {
        let frames_rendered =
            render_gif_timeline(path, &filtered, args.gif_max_frames, args.gif_frame_delay)?;
        eprintln!(
            "Animated GIF timeline with {} frame(s) written to {}",
            frames_rendered,
            path.display()
        );
    }

    match args.format {
        ArtifactFormat::Json => {
            let payload = json!({
                "summary": summary,
                "artifacts": artifacts_array,
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
        ArtifactFormat::Yaml => {
            let payload = json!({
                "summary": summary,
                "artifacts": artifacts_array,
            });
            println!("{}", serde_yaml::to_string(&payload)?);
        }
        ArtifactFormat::Human => {
            print_summary_human(&summary);
            if filtered.is_empty() {
                println!("[no artifacts]");
            } else {
                print_artifact_table(&filtered);
            }
        }
    }

    Ok(())
}

fn filter_artifacts(value: &Value, args: &ArtifactsArgs) -> Vec<Value> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };

    items
        .iter()
        .filter(|item| {
            if let Some(step_id) = &args.step_id {
                if item
                    .get("step_id")
                    .and_then(Value::as_str)
                    .map(|s| s != step_id)
                    .unwrap_or(true)
                {
                    return false;
                }
            }

            if let Some(dispatch) = &args.dispatch {
                if item
                    .get("dispatch_label")
                    .and_then(Value::as_str)
                    .map(|s| s != dispatch)
                    .unwrap_or(true)
                {
                    return false;
                }
            }

            if let Some(label) = &args.label {
                if item
                    .get("label")
                    .and_then(Value::as_str)
                    .map(|s| s != label)
                    .unwrap_or(true)
                {
                    return false;
                }
            }

            true
        })
        .cloned()
        .collect()
}

async fn extract_artifacts(dir: &PathBuf, artifacts: &[Value]) -> Result<()> {
    fs::create_dir_all(dir)
        .await
        .with_context(|| format!("failed to create extract directory {}", dir.display()))?;

    for item in artifacts {
        let label = item
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("artifact");
        let attempt = item.get("attempt").and_then(Value::as_u64).unwrap_or(0);
        let step = item.get("step_index").and_then(Value::as_u64).unwrap_or(0);
        let dispatch = item
            .get("dispatch_label")
            .and_then(Value::as_str)
            .unwrap_or("action");
        let filename_hint = item.get("filename").and_then(Value::as_str);
        let data_base64 = item
            .get("data_base64")
            .and_then(Value::as_str)
            .unwrap_or("");

        if data_base64.is_empty() {
            continue;
        }

        let bytes = match Base64.decode(data_base64) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("failed to decode artifact {}: {}", label, err);
                continue;
            }
        };

        let file_name = filename_hint
            .map(|name| name.to_string())
            .unwrap_or_else(|| {
                format!("attempt{}_step{}_{}_{}.bin", attempt, step, dispatch, label)
            });

        let path = dir.join(file_name);
        fs::write(&path, bytes)
            .await
            .with_context(|| format!("failed to write artifact {}", path.display()))?;
    }

    Ok(())
}

pub(crate) fn build_artifact_summary(items: &[Value], large_threshold: u64) -> Value {
    let total = items.len() as u64;
    let total_bytes: u64 = items
        .iter()
        .filter_map(|item| item.get("byte_len").and_then(Value::as_u64))
        .sum();
    let mut steps = HashSet::new();
    let mut dispatches = HashSet::new();
    let mut types: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut large = Vec::new();
    let mut structured = Vec::new();

    for item in items {
        if let Some(step) = item.get("step_id").and_then(Value::as_str) {
            steps.insert(step.to_string());
        }
        if let Some(dispatch) = item.get("dispatch_label").and_then(Value::as_str) {
            dispatches.insert(dispatch.to_string());
        }
        let ctype = item
            .get("content_type")
            .and_then(Value::as_str)
            .unwrap_or("application/octet-stream");
        let bytes = item.get("byte_len").and_then(Value::as_u64).unwrap_or(0);
        let entry = types.entry(ctype.to_string()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += bytes;

        if bytes >= large_threshold {
            large.push(json!({
                "step_id": item.get("step_id"),
                "dispatch_label": item.get("dispatch_label"),
                "label": item.get("label"),
                "byte_len": bytes,
                "content_type": ctype,
                "filename": item.get("filename"),
            }));
        }

        if let Some(data) = item.get("data") {
            if data.is_object() {
                structured.push(json!({
                    "step_id": item.get("step_id"),
                    "label": item.get("label"),
                    "dispatch_label": item.get("dispatch_label"),
                }));
            }
        }
    }

    json!({
        "total": total,
        "total_bytes": total_bytes,
        "steps": steps.len(),
        "dispatches": dispatches.len(),
        "types": types
            .into_iter()
            .map(|(ctype, (count, bytes))| json!({
                "content_type": ctype,
                "count": count,
                "bytes": bytes,
            }))
            .collect::<Vec<Value>>(),
        "large": large,
        "structured": structured,
    })
}

fn print_summary_human(summary: &Value) {
    println!("Artifact Summary:");
    println!("------------------");
    println!(
        "Total artifacts: {}",
        summary.get("total").and_then(Value::as_u64).unwrap_or(0)
    );
    println!(
        "Total bytes    : {}",
        summary
            .get("total_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    );
    println!(
        "Unique steps   : {}",
        summary.get("steps").and_then(Value::as_u64).unwrap_or(0)
    );
    println!(
        "Dispatch labels: {}",
        summary
            .get("dispatches")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    );
    if let Some(types) = summary.get("types").and_then(Value::as_array) {
        println!("Content types:");
        for entry in types {
            let ctype = entry
                .get("content_type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let count = entry.get("count").and_then(Value::as_u64).unwrap_or(0);
            let bytes = entry.get("bytes").and_then(Value::as_u64).unwrap_or(0);
            println!("- {:<40} count={:<4} bytes={}", ctype, count, bytes);
        }
    }
}

fn print_artifact_table(items: &[Value]) {
    for item in items {
        let attempt = item.get("attempt").and_then(Value::as_u64).unwrap_or(0);
        let step = item.get("step_index").and_then(Value::as_u64).unwrap_or(0);
        let step_id = item
            .get("step_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let dispatch = item
            .get("dispatch_label")
            .and_then(Value::as_str)
            .unwrap_or("action");
        let label = item
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("artifact");
        let content_type = item
            .get("content_type")
            .and_then(Value::as_str)
            .unwrap_or("application/octet-stream");
        let bytes = item.get("byte_len").and_then(Value::as_u64).unwrap_or(0);
        let filename = item.get("filename").and_then(Value::as_str);
        println!(
            "attempt={} step={} ({}) dispatch={} artifact={} bytes={} type={}{}",
            attempt,
            step,
            step_id,
            dispatch,
            label,
            bytes,
            content_type,
            filename
                .map(|name| format!(" filename={}", name))
                .unwrap_or_default()
        );
    }
}

async fn save_summary(path: &PathBuf, summary: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(summary)?)
        .await
        .with_context(|| format!("failed to write summary to {}", path.display()))
}

fn render_gif_timeline(
    path: &Path,
    artifacts: &[Value],
    max_frames: usize,
    delay_ms: u32,
) -> Result<usize> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
    }

    let overlay_info = extract_overlay_info(artifacts);
    let mut frames = collect_gif_frames(artifacts, max_frames, overlay_info.as_ref())?;
    if frames.is_empty() {
        bail!("no image artifacts matched the current filters; nothing to render");
    }

    let writer = File::create(path)
        .with_context(|| format!("failed to create GIF timeline {}", path.display()))?;
    let mut encoder = GifEncoder::new(BufWriter::new(writer));
    encoder
        .set_repeat(Repeat::Infinite)
        .context("failed to set GIF repeat mode")?;
    let delay = Delay::from_numer_denom_ms(delay_ms, 1);
    let frame_count = frames.len();
    for frame in frames.drain(..) {
        encoder
            .encode_frame(Frame::from_parts(frame, 0, 0, delay))
            .context("failed to encode GIF frame")?;
    }

    Ok(frame_count)
}

fn collect_gif_frames(
    artifacts: &[Value],
    max_frames: usize,
    overlay_info: Option<&OverlayInfo>,
) -> Result<Vec<RgbaImage>> {
    let mut frames = Vec::new();
    let mut target_dimensions: Option<(u32, u32)> = None;

    for item in artifacts {
        let content_type = item
            .get("content_type")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !content_type.starts_with("image/") {
            continue;
        }
        let data_base64 = match item.get("data_base64").and_then(Value::as_str) {
            Some(value) if !value.is_empty() => value,
            _ => continue,
        };
        let label = item
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("artifact");
        let bytes = Base64
            .decode(data_base64)
            .map_err(|err| anyhow!("failed to decode base64 for {label}: {err}"))?;
        let dynamic = image::load_from_memory(&bytes)
            .map_err(|err| anyhow!("failed to load image for {label}: {err}"))?;

        let (target_w, target_h) = match target_dimensions {
            Some(pair) => pair,
            None => {
                let dims = dynamic.dimensions();
                target_dimensions = Some(dims);
                dims
            }
        };

        let rgba = if dynamic.dimensions() != (target_w, target_h) {
            dynamic
                .resize_exact(target_w, target_h, FilterType::Triangle)
                .to_rgba8()
        } else {
            dynamic.to_rgba8()
        };

        let mut frame_with_overlay = rgba;
        if let Some(info) = overlay_info {
            apply_overlay(&mut frame_with_overlay, info)?;
        }
        frames.push(frame_with_overlay);
        if max_frames > 0 && frames.len() >= max_frames {
            break;
        }
    }

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::png::PngEncoder;
    use image::{ColorType, ImageBuffer, ImageEncoder, Rgba};
    use serde_json::json;

    #[test]
    fn gif_frames_ignore_non_images_and_respect_limits() {
        let png = sample_png_base64(1, 1, [255, 0, 0, 255]);
        let artifacts = vec![
            json!({
                "content_type": "text/plain",
                "data_base64": Base64.encode(b"hello"),
            }),
            json!({
                "content_type": "image/png",
                "label": "frame-1",
                "data_base64": png,
            }),
        ];

        let frames = collect_gif_frames(&artifacts, 1, None).expect("frames");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].dimensions(), (1, 1));
    }

    #[test]
    fn gif_frames_normalize_dimensions() {
        let first = sample_png_base64(2, 2, [0, 0, 0, 255]);
        let second = sample_png_base64(4, 1, [0, 255, 0, 255]);
        let artifacts = vec![
            json!({
                "content_type": "image/png",
                "label": "first",
                "data_base64": first,
            }),
            json!({
                "content_type": "image/png",
                "label": "second",
                "data_base64": second,
            }),
        ];

        let frames = collect_gif_frames(&artifacts, 0, None).expect("frames");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].dimensions(), (2, 2));
        assert_eq!(frames[1].dimensions(), (2, 2));
    }

    fn sample_png_base64(width: u32, height: u32, color: [u8; 4]) -> String {
        let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(width, height, Rgba(color));
        let mut bytes = Vec::new();
        let encoder = PngEncoder::new(&mut bytes);
        encoder
            .write_image(buffer.as_raw(), width, height, ColorType::Rgba8)
            .expect("png encode");
        Base64.encode(bytes)
    }
}
const FONT_CHAR_WIDTH: u32 = 8;
const FONT_CHAR_HEIGHT: u32 = 8;
#[derive(Debug, Clone)]
struct OverlayInfo {
    task: Option<String>,
    goal: Option<String>,
    evaluation: Option<String>,
    next_goal: Option<String>,
}

fn extract_overlay_info(artifacts: &[Value]) -> Option<OverlayInfo> {
    let info = artifacts.iter().find_map(|item| {
        let metadata = item.get("metadata")?.as_object()?;
        let agent_state = metadata.get("agent_state")?.as_object()?;
        Some(OverlayInfo {
            task: metadata
                .get("task")
                .and_then(Value::as_str)
                .map(|s| s.to_string()),
            goal: metadata
                .get("goal")
                .and_then(Value::as_str)
                .map(|s| s.to_string()),
            evaluation: agent_state
                .get("evaluation")
                .and_then(Value::as_str)
                .map(|s| s.to_string()),
            next_goal: agent_state
                .get("next_goal")
                .and_then(Value::as_str)
                .map(|s| s.to_string()),
        })
    })?;
    Some(info)
}

fn apply_overlay(image: &mut RgbaImage, info: &OverlayInfo) -> Result<()> {
    let mut y_offset = 8;
    if let Some(task) = info.task.as_deref() {
        draw_small_text(image, task, 12, y_offset);
        y_offset += FONT_CHAR_HEIGHT as i32 + 6;
    }
    if let Some(goal) = info.goal.as_deref() {
        draw_small_text(image, goal, 12, y_offset);
        y_offset += FONT_CHAR_HEIGHT as i32 + 6;
    }
    if let Some(eval) = info.evaluation.as_deref() {
        let text = format!("Eval: {}", eval);
        draw_small_text(image, &text, 12, y_offset);
        y_offset += FONT_CHAR_HEIGHT as i32 + 6;
    }
    if let Some(next) = info.next_goal.as_deref() {
        let text = format!("Next: {}", next);
        draw_small_text(image, &text, 12, y_offset);
    }
    Ok(())
}

fn draw_small_text(image: &mut RgbaImage, text: &str, x: i32, y: i32) {
    let color = Rgba([255, 255, 0, 220]);
    let mut cursor_x = x;
    for ch in text.chars() {
        if ch == '\n' {
            cursor_x = x;
            continue;
        }
        if let Some(bitmap) = BASIC_FONTS.get(ch).or_else(|| BASIC_FONTS.get('?')) {
            draw_char(image, cursor_x, y, &bitmap, color);
        }
        cursor_x += FONT_CHAR_WIDTH as i32;
        if cursor_x as u32 >= image.width().saturating_sub(FONT_CHAR_WIDTH) {
            break;
        }
    }
}

fn draw_char(image: &mut RgbaImage, x: i32, y: i32, bitmap: &[u8; 8], color: Rgba<u8>) {
    for (row, bits) in bitmap.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << col) != 0 {
                let px = x + col as i32;
                let py = y + row as i32;
                if px >= 0 && py >= 0 && (px as u32) < image.width() && (py as u32) < image.height()
                {
                    image.put_pixel(px as u32, py as u32, color);
                }
            }
        }
    }
}
