use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as Base64, Engine as _};
use clap::{Args, ValueEnum};
use serde_json::{json, Value};
use tokio::fs;
use tracing::warn;

use crate::load_run_bundle;
use crate::DEFAULT_LARGE_THRESHOLD;

#[derive(Args)]
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
