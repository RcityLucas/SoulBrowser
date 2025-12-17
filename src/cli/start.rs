use crate::cli::context::CliContext;
use anyhow::{Context, Result};
use clap::Args;
use soulbrowser_kernel::browser_impl::{BrowserConfig, L0Protocol, L1BrowserManager};
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
    let _context = ctx.app_context().await?;

    let l0 = L0Protocol::new().await?;
    let browser_config = BrowserConfig {
        browser_type: args.browser.clone(),
        headless: args.headless,
        window_size: parse_window_size(args.window_size.as_deref())?,
        devtools: args.devtools,
        ..Default::default()
    };

    let mut l1 = L1BrowserManager::new(l0, browser_config).await?;
    let browser = l1.launch_browser().await?;
    let mut page = browser
        .new_page()
        .await
        .context("Failed to create new page")?;

    if let Some(url) = args.url.clone() {
        page.navigate(&url)
            .await
            .context("Failed to navigate to URL")?;
    }

    tokio::signal::ctrl_c().await?;
    Ok(())
}

fn parse_window_size(arg: Option<&str>) -> Result<Option<(u32, u32)>> {
    match arg {
        Some(value) => {
            let parts: Vec<&str> = value.split('x').collect();
            if parts.len() != 2 {
                anyhow::bail!("Invalid window size format. Use WIDTHxHEIGHT, e.g., 1280x720");
            }
            let width = parts[0].parse::<u32>().context("Invalid width")?;
            let height = parts[1].parse::<u32>().context("Invalid height")?;
            Ok(Some((width, height)))
        }
        None => Ok(None),
    }
}
