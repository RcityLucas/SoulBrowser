use anyhow::Result;
use crate::{Config, RunArgs};

pub async fn cmd_run(args: RunArgs, config: &Config) -> Result<()> {
    crate::run::run_script(args, config).await
}
