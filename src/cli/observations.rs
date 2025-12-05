use anyhow::{bail, Context, Result};
use clap::{Args, ValueEnum};
use serde_json::Value;

use crate::app_context::get_or_create_context;
use crate::task_status::TaskLogEntry;
use crate::Config;

#[derive(Args)]
pub struct ObservationsArgs {
    /// Task ID to inspect
    pub task_id: String,

    /// Maximum number of observations to display
    #[arg(long, default_value_t = 50)]
    pub limit: usize,

    /// Filter by observation type (e.g. image, artifact)
    #[arg(long)]
    pub observation_type: Option<String>,

    /// Output format (table or json)
    #[arg(long, value_enum, default_value = "table")]
    pub format: ObservationsOutputFormat,
}

#[derive(Clone, ValueEnum, Debug)]
pub enum ObservationsOutputFormat {
    Table,
    Json,
}

pub async fn cmd_observations(args: ObservationsArgs, config: &Config) -> Result<()> {
    let context = get_or_create_context(
        "cli-observations".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await?;

    let registry = context.task_status_registry();
    let Some(history) = registry.observation_history(&args.task_id, args.limit) else {
        bail!("task '{}' not found in registry", args.task_id);
    };

    let normalized_filter = args
        .observation_type
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());

    let filtered: Vec<Value> = match normalized_filter {
        Some(ref needle) => history
            .into_iter()
            .filter(|entry| observation_matches(entry, needle))
            .collect(),
        None => history,
    };

    match args.format {
        ObservationsOutputFormat::Table => print_observation_table(&filtered),
        ObservationsOutputFormat::Json => {
            let payload = serde_json::json!({
                "task_id": args.task_id,
                "count": filtered.len(),
                "observations": filtered,
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
    }

    Ok(())
}

fn observation_matches(entry: &Value, needle: &str) -> bool {
    entry
        .get("kind")
        .and_then(Value::as_str)
        .map(|value| value.to_ascii_lowercase() == needle)
        .unwrap_or(false)
}

fn print_observation_table(entries: &[Value]) {
    println!(
        "{:<8} {:<12} {:<20} {:<32} {}",
        "INDEX", "KIND", "TIMESTAMP", "LABEL", "SUMMARY"
    );
    for (idx, entry) in entries.iter().enumerate() {
        let kind = entry
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let timestamp = entry
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let label = entry.get("label").and_then(Value::as_str).unwrap_or("—");
        let summary = entry.get("summary").and_then(Value::as_str).unwrap_or("—");
        println!(
            "{:<8} {:<12} {:<20} {:<32} {}",
            idx + 1,
            kind,
            timestamp,
            label,
            summary
        );
    }
}
