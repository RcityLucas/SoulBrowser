use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use memory_center::{normalize_note, normalize_tags, MemoryRecord, MemoryStatsSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;

use soulbrowser_kernel::app_context::get_or_create_context;
use soulbrowser_kernel::Config;

#[derive(Args)]
pub struct MemoryArgs {
    #[command(subcommand)]
    pub command: MemoryCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum MemoryCommand {
    List(MemoryListArgs),
    Add(MemoryAddArgs),
    Delete(MemoryDeleteArgs),
    Update(MemoryUpdateArgs),
    Export(MemoryExportArgs),
    Import(MemoryImportArgs),
    Stats,
}

#[derive(Args, Debug, Clone)]
pub struct MemoryListArgs {
    #[arg(long)]
    pub namespace: Option<String>,
    #[arg(long)]
    pub tag: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long, value_enum, default_value_t = MemoryOutputFormat::Table)]
    pub format: MemoryOutputFormat,
}

#[derive(Args, Debug, Clone)]
pub struct MemoryAddArgs {
    pub namespace: String,
    pub key: String,
    #[arg(long, value_delimiter = ',')]
    pub tags: Vec<String>,
    #[arg(long)]
    pub note: Option<String>,
    #[arg(long)]
    pub metadata: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct MemoryDeleteArgs {
    #[arg(value_name = "MEMORY_ID")]
    pub id: String,
}

#[derive(Args, Debug, Clone)]
pub struct MemoryUpdateArgs {
    #[arg(value_name = "MEMORY_ID")]
    pub id: String,
    #[arg(long, value_delimiter = ',')]
    pub tags: Option<Vec<String>>,
    #[arg(long)]
    pub note: Option<String>,
    #[arg(long)]
    pub metadata: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct MemoryExportArgs {
    #[arg(value_name = "MEMORY_ID")]
    pub id: String,
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long, default_value_t = true)]
    pub pretty: bool,
}

#[derive(Args, Debug, Clone)]
pub struct MemoryImportArgs {
    #[arg(value_name = "FILE")]
    pub input: PathBuf,
    #[arg(long)]
    pub namespace: Option<String>,
    #[arg(long)]
    pub key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryTemplatePayload {
    pub version: u32,
    pub namespace: String,
    pub key: String,
    pub tags: Vec<String>,
    pub note: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

impl From<&MemoryRecord> for MemoryTemplatePayload {
    fn from(record: &MemoryRecord) -> Self {
        Self {
            version: 1,
            namespace: record.namespace.clone(),
            key: record.key.clone(),
            tags: record.tags.clone(),
            note: record.note.clone(),
            metadata: record.metadata.clone(),
        }
    }
}

impl MemoryTemplatePayload {
    fn into_record(self, namespace: Option<&str>, key: Option<&str>) -> MemoryRecord {
        let mut record = MemoryRecord::new(
            namespace.unwrap_or(&self.namespace),
            key.unwrap_or(&self.key),
        );
        record.tags = self.tags;
        record.note = self.note;
        record.metadata = self.metadata;
        record
    }
}

#[derive(Clone, ValueEnum, Debug)]
pub enum MemoryOutputFormat {
    Table,
    Json,
}

pub async fn cmd_memory(args: MemoryArgs, config: &Config) -> Result<()> {
    let context = get_or_create_context(
        "cli-memory".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await?;
    let center = context.memory_center();

    match args.command {
        MemoryCommand::List(opts) => {
            let records = center.list(opts.namespace.as_deref(), opts.tag.as_deref(), opts.limit);
            match opts.format {
                MemoryOutputFormat::Table => print_memory_table(&records),
                MemoryOutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&records)?);
                }
            }
        }
        MemoryCommand::Add(opts) => {
            let mut record = MemoryRecord::new(&opts.namespace, &opts.key);
            record.tags = normalize_tags(&opts.tags);
            record.note = normalize_note(opts.note.as_deref());
            if let Some(metadata_raw) = opts.metadata.as_deref() {
                let value: Value =
                    serde_json::from_str(metadata_raw).context("metadata must be valid JSON")?;
                record.metadata = Some(value);
            }
            let stored = center.store(record);
            println!(
                "Stored memory '{}' (namespace={}, tags={})",
                stored.id,
                stored.namespace,
                if stored.tags.is_empty() {
                    "-".to_string()
                } else {
                    stored.tags.join(",")
                }
            );
        }
        MemoryCommand::Delete(opts) => {
            if center.remove_by_id(&opts.id).is_some() {
                println!("Deleted memory record {}", opts.id);
            } else {
                println!("No memory record found for id {}", opts.id);
            }
        }
        MemoryCommand::Update(opts) => {
            if opts.tags.is_none() && opts.note.is_none() && opts.metadata.is_none() {
                println!("No update fields provided.");
                return Ok(());
            }

            let metadata_value = match opts.metadata.as_deref() {
                Some(raw) if raw.trim().is_empty() => Some(Value::Null),
                Some(raw) => match serde_json::from_str::<Value>(raw) {
                    Ok(value) => Some(value),
                    Err(err) => {
                        eprintln!("metadata must be valid JSON: {err}");
                        return Ok(());
                    }
                },
                None => None,
            };

            let tags_patch = opts.tags.as_ref().map(|tags| normalize_tags(tags));
            let note_field_provided = opts.note.is_some();
            let note_value = normalize_note(opts.note.as_deref());
            let metadata_field_provided = metadata_value.is_some();
            let metadata_patch = metadata_value.clone();

            let updated = center.update_record(&opts.id, |record| {
                if let Some(tags) = tags_patch.as_ref() {
                    record.tags = tags.clone();
                }
                if note_field_provided {
                    record.note = note_value.clone();
                }
                if metadata_field_provided {
                    if let Some(meta) = metadata_patch.as_ref() {
                        if meta.is_null() {
                            record.metadata = None;
                        } else {
                            record.metadata = Some(meta.clone());
                        }
                    } else {
                        record.metadata = None;
                    }
                }
            });

            if let Some(record) = updated {
                println!(
                    "Updated memory '{}' (namespace={}, tags={})",
                    record.id,
                    record.namespace,
                    if record.tags.is_empty() {
                        "-".to_string()
                    } else {
                        record.tags.join(",")
                    }
                );
            } else {
                println!("No memory record found for id {}", opts.id);
            }
        }
        MemoryCommand::Export(opts) => {
            let Some(record) = center.get_by_id(&opts.id) else {
                bail!("No memory record found for id {}", opts.id);
            };
            let payload = MemoryTemplatePayload::from(&record);
            let json = if opts.pretty {
                serde_json::to_string_pretty(&payload)?
            } else {
                serde_json::to_string(&payload)?
            };
            if let Some(path) = opts.output.as_ref() {
                fs::write(path, json.as_bytes())
                    .await
                    .with_context(|| format!("Failed to write template to {}", path.display()))?;
                println!("Exported memory '{}' to {}", record.id, path.display());
            } else {
                println!("{}", json);
            }
        }
        MemoryCommand::Import(opts) => {
            let bytes = fs::read(&opts.input)
                .await
                .with_context(|| format!("Failed to read template {}", opts.input.display()))?;
            let payload: MemoryTemplatePayload = serde_json::from_slice(&bytes)
                .with_context(|| format!("Template JSON invalid in {}", opts.input.display()))?;
            if payload.version != 1 {
                println!(
                    "Warning: template version {} not recognized (expected 1)",
                    payload.version
                );
            }
            let record = payload.into_record(opts.namespace.as_deref(), opts.key.as_deref());
            let stored = center.store(record);
            println!(
                "Imported template '{}' (namespace={}, key={})",
                stored.id, stored.namespace, stored.key
            );
        }
        MemoryCommand::Stats => {
            let snapshot = center.stats_snapshot();
            print_memory_stats(&snapshot);
        }
    }

    Ok(())
}

fn print_memory_table(records: &[MemoryRecord]) {
    println!(
        "{:<36} {:<16} {:<24} {:<10} {}",
        "ID", "NAMESPACE", "KEY", "TAGS", "NOTE"
    );
    for record in records {
        let tags_display = if record.tags.is_empty() {
            "-".to_string()
        } else {
            record.tags.join(",")
        };
        println!(
            "{:<36} {:<16} {:<24} {:<10} {}",
            record.id,
            record.namespace,
            record.key,
            tags_display,
            record
                .note
                .as_deref()
                .map(|note| note.chars().take(60).collect::<String>())
                .unwrap_or_else(|| "".to_string())
        );
    }
}

fn print_memory_stats(stats: &MemoryStatsSnapshot) {
    println!("Memory Center Stats:");
    println!("- Total queries: {}", stats.total_queries);
    println!("- Hits: {}", stats.hit_queries);
    println!("- Misses: {}", stats.miss_queries);
    println!("- Hit rate: {:.2}%", stats.hit_rate * 100.0);
    println!("- Records stored: {}", stats.stored_records);
    println!("- Records deleted: {}", stats.deleted_records);
    println!("- Current records: {}", stats.current_records);
    println!("- Template uses: {}", stats.template_uses);
    println!("- Template successes: {}", stats.template_successes);
    println!(
        "- Template success rate: {:.2}%",
        stats.template_success_rate * 100.0
    );
}
