use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Args;

use crate::cli::context::CliContext;
use soulbrowser_kernel::types::BrowserType;

#[derive(Args, Clone, Debug)]
pub struct RecordArgs {
    /// Session name
    pub name: String,

    /// Browser type to use
    #[arg(short, long, default_value = "chromium")]
    pub browser: BrowserType,

    /// Start URL
    #[arg(short, long)]
    pub url: Option<String>,

    /// Recording output directory
    #[arg(short, long)]
    pub output_dir: Option<PathBuf>,

    /// Enable screenshot recording
    #[arg(long)]
    pub screenshots: bool,

    /// Enable video recording
    #[arg(long)]
    pub video: bool,

    /// Record network activity
    #[arg(long)]
    pub network: bool,

    /// Record performance metrics
    #[arg(long)]
    pub performance: bool,
}

pub async fn cmd_record(args: RecordArgs, ctx: &CliContext) -> Result<()> {
    let _ = (args, ctx);
    bail!("record command has been retired; use unified serve/gateway flows instead")
}
