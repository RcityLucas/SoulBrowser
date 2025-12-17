use std::env;
use std::path::{Path, PathBuf};

use crate::cli::context::CliContext;
use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use clap::Args;
use perceiver_hub::models::MultiModalPerception;
use serde::Serialize;
use soulbrowser_kernel::perception_service::{PerceptionJob, PerceptionService};
use tokio::fs;

#[derive(Args, Clone, Debug)]
pub struct PerceiveArgs {
    /// URL to analyze
    #[arg(long)]
    pub url: String,

    /// Enable visual perception (screenshots, visual metrics)
    #[arg(long)]
    pub visual: bool,

    /// Enable semantic perception (content classification, language detection)
    #[arg(long)]
    pub semantic: bool,

    /// Enable structural perception (DOM/AX tree analysis)
    #[arg(long)]
    pub structural: bool,

    /// Enable all perception modes (visual + semantic + structural)
    #[arg(long)]
    pub all: bool,

    /// Capture screenshot to file
    #[arg(long)]
    pub screenshot: Option<PathBuf>,

    /// Output perception results to JSON file
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Show cross-modal insights
    #[arg(long)]
    pub insights: bool,

    /// Override Chrome/Chromium executable path
    #[arg(long)]
    pub chrome_path: Option<PathBuf>,

    /// Run Chrome with a visible window instead of headless mode
    #[arg(long)]
    pub headful: bool,

    /// Attach to an existing Chrome DevTools websocket
    #[arg(long)]
    pub ws_url: Option<String>,

    /// Analysis timeout in seconds
    #[arg(long, default_value = "30")]
    pub timeout: u64,
}

pub async fn cmd_perceive(args: PerceiveArgs, ctx: &CliContext) -> Result<()> {
    let context = ctx.app_context().await?;
    let service = PerceptionService::with_app_context(&context);

    let (enable_structural, enable_visual, enable_semantic) = resolve_modes(&args);
    let ws_url = args
        .ws_url
        .clone()
        .or_else(|| env::var("SOULBROWSER_WS_URL").ok());
    let allow_pooling = ws_url.is_none();
    let capture_screenshot = args.screenshot.is_some() || enable_visual;

    let job = PerceptionJob {
        url: args.url.clone(),
        enable_structural,
        enable_visual,
        enable_semantic,
        enable_insights: args.insights,
        capture_screenshot,
        timeout_secs: args.timeout,
        chrome_path: args.chrome_path.clone(),
        ws_url,
        headful: args.headful,
        viewport: None,
        cookies: Vec::new(),
        inject_script: None,
        allow_pooling,
    };

    println!("Starting perception for {}", args.url);
    let output = service.perceive(job).await?;

    print_summary(&args.url, &output.perception);

    if !output.log_lines.is_empty() {
        println!("\n[perception log]");
        for line in &output.log_lines {
            println!("- {}", line);
        }
    }

    if let Some(path) = args.output.as_ref() {
        let payload = PerceptionFilePayload {
            url: &args.url,
            perception: &output.perception,
            logs: &output.log_lines,
            screenshot_base64: output.screenshot.as_ref().map(|bytes| BASE64.encode(bytes)),
        };
        write_json_output(path, &payload).await?;
        println!("Saved perception JSON to {}", path.display());
    }

    if let Some(path) = args.screenshot.as_ref() {
        if let Some(bytes) = &output.screenshot {
            write_binary_file(path, bytes).await?;
            println!("Saved screenshot to {}", path.display());
        } else {
            println!("Screenshot data unavailable; enable visual mode or capture explicitly");
        }
    }

    Ok(())
}

fn resolve_modes(args: &PerceiveArgs) -> (bool, bool, bool) {
    let mut enable_structural = args.structural;
    let mut enable_visual = args.visual;
    let mut enable_semantic = args.semantic;
    if args.all || (!enable_structural && !enable_visual && !enable_semantic) {
        enable_structural = true;
        enable_visual = true;
        enable_semantic = true;
    }
    (enable_structural, enable_visual, enable_semantic)
}

fn print_summary(url: &str, perception: &MultiModalPerception) {
    println!("\n=== Perception summary ===");
    println!("URL: {}", url);
    println!("Confidence: {:.2}", perception.confidence);

    let structural = &perception.structural;
    println!(
        "Structural → nodes: {}, interactive: {}, forms: {}, navigation: {}",
        structural.dom_node_count,
        structural.interactive_element_count,
        structural.has_forms,
        structural.has_navigation
    );

    match &perception.visual {
        Some(visual) => {
            println!(
                "Visual → screenshot: {}, avg contrast: {:.2}, viewport usage: {:.0}%",
                visual.screenshot_id,
                visual.avg_contrast,
                visual.viewport_utilization * 100.0
            );
        }
        None => println!("Visual → disabled"),
    }

    match &perception.semantic {
        Some(semantic) => {
            println!(
                "Semantic → language: {} ({:.0}%), intent: {:?}",
                semantic.language,
                semantic.language_confidence * 100.0,
                semantic.intent
            );
            if !semantic.summary.is_empty() {
                println!("  Summary: {}", semantic.summary);
            }
            if !semantic.keywords.is_empty() {
                println!("  Keywords: {}", semantic.keywords.join(", "));
            }
        }
        None => println!("Semantic → disabled"),
    }

    if !perception.insights.is_empty() {
        println!("\nInsights:");
        for insight in &perception.insights {
            println!("- [{:?}] {}", insight.insight_type, insight.description);
        }
    }
}

async fn write_json_output(path: &Path, payload: &PerceptionFilePayload<'_>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(payload)?;
    fs::write(path, json)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

async fn write_binary_file(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, data)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[derive(Serialize)]
struct PerceptionFilePayload<'a> {
    url: &'a str,
    perception: &'a MultiModalPerception,
    logs: &'a [String],
    screenshot_base64: Option<String>,
}
