use anyhow::Result;
use clap::Parser;
use tracing::{error, info};

use super::context::CliContext;
use super::dispatch::dispatch;
use super::env::CliArgs;
use super::runtime::{
    apply_runtime_overrides, init_logging, load_config, load_local_env_overrides, LoadedConfig,
};
use soulbrowser_kernel::metrics;

pub async fn run() -> Result<()> {
    load_local_env_overrides();
    let cli = CliArgs::parse();

    init_logging(&cli.log_level, cli.debug)?;
    let _metrics_server = metrics::spawn_metrics_server(cli.metrics_port);

    info!("Starting SoulBrowser v{}", env!("CARGO_PKG_VERSION"));

    let loaded_config = load_config(cli.config.as_ref()).await?;
    apply_runtime_overrides(&loaded_config.config);
    let LoadedConfig { config, path } = loaded_config;
    let cli_context = CliContext::new(config, path, cli.metrics_port);

    match dispatch(&cli, &cli_context).await {
        Ok(()) => {
            info!("Command completed successfully");
            Ok(())
        }
        Err(err) => {
            error!("Command failed: {}", err);
            Err(err)
        }
    }
}
