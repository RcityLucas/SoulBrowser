use anyhow::Result;
use clap::{Args, Subcommand};
use serde::Deserialize;
use serde_json::to_string_pretty;

use soulbrowser_kernel::task_store::TaskPlanStore;
use soulbrowser_kernel::Config;

#[derive(Args)]
pub struct TasksArgs {
    #[command(subcommand)]
    pub command: TaskCommand,
}

#[derive(Subcommand)]
pub enum TaskCommand {
    /// List all persisted task plans
    List,
    /// Show a specific task plan
    Show {
        /// Task identifier (UUID)
        task_id: String,
    },
    /// Watch a task's structured outputs (live refresh)
    Watch {
        /// Task identifier (UUID)
        task_id: String,
    },
}

pub async fn cmd_tasks(args: TasksArgs, config: &Config) -> Result<()> {
    let store = TaskPlanStore::new(config.output_dir.clone());
    match args.command {
        TaskCommand::List => {
            let mut summaries = store.list_plan_summaries().await?;
            summaries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            if summaries.is_empty() {
                println!(
                    "No task plans found under {}",
                    config.output_dir.join("tasks").display()
                );
                return Ok(());
            }
            println!(
                "{:<38} {:<25} {:<10} {}",
                "TASK ID", "CREATED", "SOURCE", "PROMPT"
            );
            for summary in summaries {
                println!(
                    "{:<38} {:<25} {:<10} {}",
                    summary.task_id,
                    summary.created_at,
                    summary.source,
                    summary.prompt.replace('\n', " ").trim()
                );
            }
            Ok(())
        }
        TaskCommand::Show { task_id } => {
            let record = store.load_plan(&task_id).await?;
            println!(
                "Task {} ({} from {})",
                record.task_id, record.source, record.created_at
            );
            println!("Prompt: {}", record.prompt);
            if !record.constraints.is_empty() {
                println!("Constraints: {}", record.constraints.join(", "));
            }
            if let Some(url) = record.current_url.as_ref() {
                println!("Seed URL: {url}");
            }
            println!("\nPlan JSON:\n{}", to_string_pretty(&record.plan)?);
            println!("\nFlow JSON:\n{}", to_string_pretty(&record.flow)?);
            Ok(())
        }
        TaskCommand::Watch { task_id } => watch_task_outputs(&task_id, config).await,
    }
}

async fn watch_task_outputs(task_id: &str, config: &Config) -> Result<()> {
    use std::io::ErrorKind;
    use std::time::SystemTime;
    use tokio::time::{sleep, Duration};

    let artifacts_dir = config.output_dir.join("artifacts").join(task_id);
    println!(
        "Watching structured artifacts for task {} under {}",
        task_id,
        artifacts_dir.display()
    );
    let mut last_change: Option<SystemTime> = None;
    loop {
        match tokio::fs::read_dir(&artifacts_dir).await {
            Ok(mut entries) => {
                while let Some(entry) = entries.next_entry().await? {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if !name.contains("metal_price_v1") {
                        continue;
                    }
                    let metadata = entry.metadata().await.ok();
                    let modified = metadata
                        .and_then(|meta| meta.modified().ok())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    if last_change.as_ref() == Some(&modified) {
                        continue;
                    }
                    let bytes = tokio::fs::read(entry.path()).await?;
                    match serde_json::from_slice::<MetalPriceFile>(&bytes) {
                        Ok(file) => {
                            println!("\n=== metal_price_v1 ({}) ===", name);
                            print_metal_price_table(&file);
                            last_change = Some(modified);
                        }
                        Err(err) => {
                            eprintln!("Failed to parse {}: {}", entry.path().display(), err);
                        }
                    }
                }
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
        sleep(Duration::from_secs(2)).await;
    }
}

fn print_metal_price_table(file: &MetalPriceFile) {
    if file.items.is_empty() {
        println!("(no metal price entries yet)");
        return;
    }
    println!(
        "{:<10} {:<12} {:<14} {:>12} {:>10} {:<16}",
        "金属", "合约", "市场", "价格", "涨跌", "时间"
    );
    for item in &file.items {
        let delta = format_delta(item.change);
        println!(
            "{:<10} {:<12} {:<14} {:>12} {:>10} {:<16}",
            item.metal.as_deref().unwrap_or("-"),
            item.contract.as_deref().unwrap_or("-"),
            item.market.as_deref().unwrap_or("-"),
            format_price_value(item.price),
            delta,
            item.as_of.as_deref().unwrap_or("-")
        );
    }
    if let Some(url) = file.source_url.as_deref() {
        println!("来源: {}", url);
    }
    if let Some(ts) = file.captured_at.as_deref() {
        println!("捕获时间: {}", ts);
    }
}

fn format_price_value(value: Option<f64>) -> String {
    value
        .map(|num| format!("{:.2}", num))
        .unwrap_or_else(|| "-".to_string())
}

fn format_delta(value: Option<f64>) -> String {
    value
        .map(|num| {
            if num > 0.0 {
                format!("+{:.0}", num)
            } else if num < 0.0 {
                format!("{:.0}", num)
            } else {
                "0".to_string()
            }
        })
        .unwrap_or_else(|| "-".to_string())
}

#[derive(Debug, Deserialize)]
struct MetalPriceFile {
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    captured_at: Option<String>,
    #[serde(default)]
    items: Vec<MetalPriceRow>,
}

#[derive(Debug, Deserialize)]
struct MetalPriceRow {
    #[serde(default)]
    metal: Option<String>,
    #[serde(default)]
    contract: Option<String>,
    #[serde(default)]
    market: Option<String>,
    #[serde(default)]
    price: Option<f64>,
    #[serde(default)]
    change: Option<f64>,
    #[serde(default)]
    as_of: Option<String>,
}
