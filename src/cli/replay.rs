use anyhow::{bail, Result};
use clap::Args;
use soulbrowser_kernel::types::BrowserType;
use soulbrowser_kernel::Config;

#[derive(Args, Clone, Debug)]
pub struct ReplayArgs {
    /// Recording file or session name
    pub recording: String,

    /// Browser type to use
    #[arg(short, long, default_value = "chromium")]
    pub browser: BrowserType,

    /// Playback speed multiplier
    #[arg(long, default_value = "1.0")]
    pub speed: f64,

    /// Enable headless mode
    #[arg(long)]
    pub headless: bool,

    /// Stop on first error
    #[arg(long)]
    pub fail_fast: bool,
}

pub async fn cmd_replay(_args: ReplayArgs, _config: &Config) -> Result<()> {
    bail!("replay command is temporarily unavailable after the CLI refactor")
}
