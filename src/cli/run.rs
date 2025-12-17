use anyhow::{bail, Result};
use clap::Args;
use soulbrowser_kernel::types::BrowserType;
use soulbrowser_kernel::Config;
use std::path::PathBuf;

#[derive(Args, Clone, Debug)]
pub struct RunArgs {
    /// Script file to execute
    pub script: PathBuf,

    /// Browser type to use
    #[arg(short, long, default_value = "chromium")]
    pub browser: BrowserType,

    /// Enable headless mode
    #[arg(long)]
    pub headless: bool,

    /// Script parameters (key=value)
    #[arg(short, long)]
    pub param: Vec<String>,

    /// Generate comparison report
    #[arg(long)]
    pub compare: bool,
}

pub async fn cmd_run(_args: RunArgs, _config: &Config) -> Result<()> {
    bail!("run command is temporarily unavailable after the CLI refactor")
}
