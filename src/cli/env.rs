use clap::Parser;
use std::path::PathBuf;

use super::commands::Commands;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct CliArgs {
    /// Configuration file path
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Log level
    #[arg(short, long, default_value = "info")]
    pub log_level: String,

    /// Enable debug mode
    #[arg(short, long)]
    pub debug: bool,

    /// Output format
    #[arg(short, long, default_value = "human")]
    pub output: crate::cli::output::OutputFormat,

    /// Metrics server port (set to 0 to disable)
    #[arg(long, default_value_t = 9090)]
    pub metrics_port: u16,

    #[command(subcommand)]
    pub command: Commands,
}
