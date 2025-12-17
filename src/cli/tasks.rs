use anyhow::Result;
use clap::{Args, Subcommand};
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
    }
}
