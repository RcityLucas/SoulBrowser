use anyhow::{bail, Result};
use clap::{Args, Subcommand, ValueEnum};
use soulbrowser_kernel::Config;
use std::path::PathBuf;

#[derive(Args, Clone, Debug)]
pub struct ExportArgs {
    /// Data type to export
    #[command(subcommand)]
    pub data_type: ExportType,
}

#[derive(Subcommand, Clone, Debug)]
pub enum ExportType {
    /// Export performance metrics
    Metrics {
        /// Session or recording name
        session: String,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Export format
        #[arg(short, long, default_value = "json")]
        format: DataFormat,
    },

    /// Export interaction timeline
    Timeline {
        /// Session or recording name
        session: String,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Export format
        #[arg(short, long, default_value = "html")]
        format: TimelineFormat,
    },

    /// Export automation scripts
    Script {
        /// Session or recording name
        session: String,

        /// Target language
        #[arg(short, long, default_value = "javascript")]
        language: ScriptLanguage,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, ValueEnum)]
pub enum DataFormat {
    Json,
    Csv,
    Excel,
    Yaml,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum TimelineFormat {
    Html,
    Json,
    Svg,
    Pdf,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum ScriptLanguage {
    JavaScript,
    TypeScript,
    Python,
    Rust,
    Go,
}

pub async fn cmd_export(_args: ExportArgs, _config: &Config) -> Result<()> {
    bail!("export command is temporarily unavailable after the CLI refactor")
}
