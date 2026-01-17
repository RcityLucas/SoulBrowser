use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Subcommand};
use soulbrowser_kernel::tool_registry::ToolDescriptor;
use tokio::fs;

use crate::cli::context::CliContext;

#[derive(Args, Clone, Debug)]
pub struct ToolsArgs {
    #[command(subcommand)]
    pub command: ToolsCommand,
}

#[derive(Subcommand, Clone, Debug)]
pub enum ToolsCommand {
    /// List all registered tools (built-in + dynamic)
    List(ToolsListArgs),
    /// Register tools from a JSON file (single object or array)
    Register(ToolsRegisterArgs),
    /// Remove a tool by identifier
    Remove(ToolsRemoveArgs),
}

#[derive(Args, Clone, Debug)]
pub struct ToolsListArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Clone, Debug)]
pub struct ToolsRegisterArgs {
    /// Path to a JSON file describing one or more tools
    #[arg(long)]
    pub file: PathBuf,
    /// Skip persisting descriptors into config/tools
    #[arg(long)]
    pub no_persist: bool,
}

#[derive(Args, Clone, Debug)]
pub struct ToolsRemoveArgs {
    /// Tool identifier to remove
    #[arg(long)]
    pub id: String,
    /// Remove persisted descriptor from config/tools if present
    #[arg(long)]
    pub purge: bool,
}

pub async fn cmd_tools(args: ToolsArgs, ctx: &CliContext) -> Result<()> {
    match args.command {
        ToolsCommand::List(list_args) => cmd_list(list_args, ctx).await,
        ToolsCommand::Register(register_args) => cmd_register(register_args, ctx).await,
        ToolsCommand::Remove(remove_args) => cmd_remove(remove_args, ctx).await,
    }
}

async fn cmd_list(args: ToolsListArgs, ctx: &CliContext) -> Result<()> {
    let app_context = ctx.app_context().await?;
    let registry = app_context.tool_registry();
    ctx.ensure_tool_registry_loaded(registry.clone()).await?;

    let entries = registry.list();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        if entries.is_empty() {
            println!("[no tools registered]");
            return Ok(());
        }
        println!("{:<32} {:<20} {:<8}", "ID", "Label", "Priority");
        println!("{}", "-".repeat(64));
        for entry in entries {
            println!("{:<32} {:<20} {:<8}", entry.id, entry.label, entry.priority);
        }
    }
    Ok(())
}

async fn cmd_register(args: ToolsRegisterArgs, ctx: &CliContext) -> Result<()> {
    let app_context = ctx.app_context().await?;
    let registry = app_context.tool_registry();
    ctx.ensure_tool_registry_loaded(registry.clone()).await?;

    let descriptors = registry
        .load_from_path(&args.file)
        .map_err(|err| anyhow!(err))?;
    if descriptors.is_empty() {
        println!("No tools defined in {}", args.file.display());
        return Ok(());
    }
    println!(
        "Registered {} tool(s) from {}",
        descriptors.len(),
        args.file.display()
    );
    if !args.no_persist {
        persist_descriptors(ctx, &descriptors).await?;
    }
    Ok(())
}

async fn cmd_remove(args: ToolsRemoveArgs, ctx: &CliContext) -> Result<()> {
    let app_context = ctx.app_context().await?;
    let registry = app_context.tool_registry();
    ctx.ensure_tool_registry_loaded(registry.clone()).await?;

    if registry.remove(&args.id) {
        println!("Removed tool '{}'", args.id);
        if args.purge {
            purge_descriptor_file(ctx, &args.id).await?;
        }
        Ok(())
    } else {
        Err(anyhow!("Tool '{}' not found", args.id))
    }
}

async fn persist_descriptors(ctx: &CliContext, descriptors: &[ToolDescriptor]) -> Result<()> {
    if descriptors.is_empty() {
        return Ok(());
    }
    let dir = config_tools_dir(ctx);
    fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("failed to create tools directory {}", dir.display()))?;
    for descriptor in descriptors {
        let filename = format!("{}.json", sanitize_id(&descriptor.id));
        let path = dir.join(filename);
        let payload = serde_json::to_vec_pretty(descriptor)?;
        fs::write(&path, payload)
            .await
            .with_context(|| format!("failed to write tool descriptor {}", path.display()))?;
    }
    Ok(())
}

async fn purge_descriptor_file(ctx: &CliContext, id: &str) -> Result<()> {
    let dir = config_tools_dir(ctx);
    if !dir.exists() {
        return Ok(());
    }
    let path = dir.join(format!("{}.json", sanitize_id(id)));
    if path.exists() {
        fs::remove_file(&path)
            .await
            .with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

fn sanitize_id(id: &str) -> String {
    id.replace(
        |c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_',
        "_",
    )
}

fn config_tools_dir(ctx: &CliContext) -> PathBuf {
    ctx.config_path()
        .parent()
        .map(|path| path.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tools")
}
