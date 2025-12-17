use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use serde_json::Value;
use tokio::fs;

use soulbrowser_kernel::structured_output;

#[derive(Args)]
pub struct SchemaArgs {
    #[command(subcommand)]
    pub command: SchemaCommand,
}

#[derive(Subcommand, Debug)]
pub enum SchemaCommand {
    /// Validate a JSON file against a known schema id
    Lint {
        /// Schema identifier (e.g., news_brief_v1)
        schema: String,
        /// JSON file to validate
        #[arg(long, value_name = "FILE")]
        file: PathBuf,
    },
}

pub async fn cmd_schema(args: SchemaArgs) -> Result<()> {
    match args.command {
        SchemaCommand::Lint { schema, file } => {
            let data = fs::read_to_string(&file)
                .await
                .with_context(|| format!("reading {}", file.display()))?;
            let value: Value = serde_json::from_str(&data)
                .with_context(|| format!("parsing {}", file.display()))?;
            structured_output::validate_structured_output(&schema, &value)
                .with_context(|| format!("schema '{}' validation failed", schema))?;
            println!("Schema '{}' validated for {}", schema, file.display());
        }
    }
    Ok(())
}
