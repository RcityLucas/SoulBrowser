use crate::types::BrowserType;
use crate::StartArgs;
use crate::{
    app_context::get_or_create_context,
    browser_impl::{BrowserConfig, L0Protocol, L1BrowserManager},
    Config,
};
use anyhow::{Context, Result};

pub async fn cmd_start(args: StartArgs, config: &Config) -> Result<()> {
    let _context = get_or_create_context(
        "cli".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;

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
