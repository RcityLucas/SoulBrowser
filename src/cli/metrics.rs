use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use serde_json::Value;
use tokio::fs;

use crate::compute_metrics_from_report;

#[derive(Args)]
pub struct MetricsArgs {
    #[command(subcommand)]
    pub command: MetricsCommand,
}

#[derive(Subcommand, Debug)]
pub enum MetricsCommand {
    /// Summarize wait/run durations from a flow execution report JSON
    Execution(ExecutionMetricsArgs),
}

#[derive(Args, Debug)]
pub struct ExecutionMetricsArgs {
    /// Path to a JSON file produced by --save-run (or a standalone execution report)
    #[arg(long, value_name = "FILE")]
    pub report: PathBuf,
}

pub async fn cmd_metrics(args: MetricsArgs) -> Result<()> {
    match args.command {
        MetricsCommand::Execution(exec) => cmd_metrics_execution(exec).await,
    }
}

async fn cmd_metrics_execution(args: ExecutionMetricsArgs) -> Result<()> {
    let data = fs::read_to_string(&args.report)
        .await
        .with_context(|| format!("reading report {}", args.report.display()))?;
    let value: Value = serde_json::from_str(&data)
        .with_context(|| format!("parsing report {}", args.report.display()))?;

    let report = value
        .get("execution")
        .or_else(|| value.get("flow").and_then(|flow| flow.get("execution")))
        .unwrap_or(&value);

    let metrics = compute_metrics_from_report(report)?;
    println!("Execution Metrics Summary");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Steps succeeded: {}", metrics.succeeded_steps);
    println!("Steps failed:    {}", metrics.failed_steps);
    println!("Dispatch count:  {}", metrics.dispatch_count);
    println!("Total wait (ms): {}", metrics.total_wait_ms);
    println!("Total run  (ms): {}", metrics.total_run_ms);
    println!(
        "Avg wait/run (ms): {:.2} / {:.2}",
        metrics.avg_wait_ms, metrics.avg_run_ms
    );
    println!(
        "Max wait/run (ms): {} / {}",
        metrics.max_wait_ms, metrics.max_run_ms
    );
    Ok(())
}
