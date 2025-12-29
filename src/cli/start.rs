use crate::cli::context::CliContext;
use anyhow::{bail, Result};
use clap::Args;
use soulbrowser_kernel::types::BrowserType;

#[derive(Args, Clone, Debug)]
pub struct StartArgs {
    /// Browser type to launch
    #[arg(short, long, default_value = "chromium")]
    pub browser: BrowserType,

    /// Start URL
    #[arg(short, long)]
    pub url: Option<String>,

    /// Browser window size
    #[arg(long, value_name = "WIDTHxHEIGHT")]
    pub window_size: Option<String>,

    /// Enable headless mode
    #[arg(long)]
    pub headless: bool,

    /// Enable developer tools
    #[arg(long)]
    pub devtools: bool,

    /// Session name for saving
    #[arg(long)]
    pub session_name: Option<String>,

    /// Enable soul (AI) assistance
    #[arg(long)]
    pub soul: bool,

    /// Soul model to use
    #[arg(long, default_value = "gpt-4")]
    pub soul_model: String,

    /// Route through the L1 unified kernel scheduler
    #[arg(long)]
    pub unified_kernel: bool,
}

pub async fn cmd_start(args: StartArgs, ctx: &CliContext) -> Result<()> {
    let _ = (args, ctx);
    bail!("start command has been retired; use `soulbrowser serve --surface console` instead")
}
