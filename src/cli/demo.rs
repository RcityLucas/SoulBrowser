use anyhow::{bail, Result};
use clap::Args;
use std::path::PathBuf;

#[derive(Args, Clone, Debug)]
pub struct DemoArgs {
    /// URL to open in the demo session
    #[arg(long, default_value = "https://www.wikipedia.org/")]
    pub url: String,

    /// Seconds to wait for the initial page/session wiring
    #[arg(long, default_value_t = 30)]
    pub startup_timeout: u64,

    /// Seconds to continue streaming events after DOM ready
    #[arg(long, default_value_t = 5)]
    pub hold_after_ready: u64,

    /// Optional path to write a PNG screenshot after navigation
    #[arg(long)]
    pub screenshot: Option<PathBuf>,

    /// Override Chrome/Chromium executable path (defaults to SOULBROWSER_CHROME or system path)
    #[arg(long)]
    pub chrome_path: Option<PathBuf>,

    /// Run Chrome with a visible window instead of headless mode
    #[arg(long)]
    pub headful: bool,

    /// Attach to an existing Chrome DevTools websocket instead of launching a new instance
    #[arg(long)]
    pub ws_url: Option<String>,

    /// CSS selector for the input field to populate during the demo
    #[arg(long, default_value = "#searchInput")]
    pub input_selector: String,

    /// Text to type into the input field
    #[arg(long, default_value = "SoulBrowser")]
    pub input_text: String,

    /// CSS selector for the submit button; if omitted no click is issued
    #[arg(long, default_value = "button.pure-button")]
    pub submit_selector: String,

    /// Skip the submit click step even if a selector is provided
    #[arg(long)]
    pub skip_submit: bool,
}

pub async fn cmd_demo_real(_args: DemoArgs) -> Result<()> {
    bail!("demo command is temporarily unavailable after the CLI refactor")
}
