use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{routing::get, Router};
use clap::Args;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use super::{artifacts::build_artifact_summary, run_bundle::load_run_bundle};
use crate::cli::constants::DEFAULT_LARGE_THRESHOLD;
use soulbrowser_kernel::CONSOLE_HTML;

#[derive(Args, Clone, Debug)]
pub struct ConsoleArgs {
    /// Path to the saved run bundle produced by --save-run
    #[arg(long, value_name = "FILE")]
    pub input: PathBuf,

    /// Output file for the console payload (prints to stdout if omitted)
    #[arg(long, value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Produce pretty-printed JSON when writing to stdout
    #[arg(long)]
    pub pretty: bool,

    /// Serve a lightweight Web Console preview instead of printing JSON
    #[arg(long)]
    pub serve: bool,

    /// Port to bind when running with --serve
    #[arg(long, default_value_t = 8710)]
    pub port: u16,
}

pub async fn cmd_console(args: ConsoleArgs) -> Result<()> {
    let bundle = load_run_bundle(&args.input).await?;

    let execution = bundle
        .get("execution")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let plans = bundle
        .get("plans")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let state_events = bundle
        .get("state_events")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let artifacts_value = bundle
        .get("artifacts")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));

    let artifacts_array = artifacts_value.clone();
    let items = artifacts_value.as_array().cloned().unwrap_or_default();
    let summary = build_artifact_summary(&items, DEFAULT_LARGE_THRESHOLD);
    let overlays = build_overlays(&items);

    let payload = json!({
        "plans": plans,
        "execution": execution,
        "state_events": state_events,
        "artifacts": {
            "summary": summary,
            "items": artifacts_array,
        },
        "overlays": overlays,
    });

    if args.serve {
        serve_console(args.port, payload).await?;
        return Ok(());
    }

    if let Some(path) = &args.output {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("failed to create output directory {}", parent.display())
            })?;
        }
        tokio::fs::write(path, serde_json::to_vec_pretty(&payload)?)
            .await
            .with_context(|| format!("failed to write console payload to {}", path.display()))?;
        println!("Console bundle written to {}", path.display());
    } else if args.pretty {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("{}", serde_json::to_string(&payload)?);
    }

    Ok(())
}

async fn serve_console(port: u16, payload: Value) -> Result<()> {
    let shared = Arc::new(payload);
    let data = shared.clone();

    let router = Router::new()
        .route("/", get(|| async { axum::response::Html(CONSOLE_HTML) }))
        .route(
            "/data",
            get(move || {
                let clone = data.clone();
                async move { axum::Json(clone.as_ref().clone()) }
            }),
        );

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind console server on {}", addr))?;
    println!("Console preview available at http://{}", addr);
    axum::serve(listener, router.into_make_service())
        .await
        .context("console server exited unexpectedly")?;
    Ok(())
}

fn build_overlays(items: &[Value]) -> Value {
    let mut overlays = Vec::new();
    for item in items {
        if let Some(overlay) = item.get("overlay") {
            overlays.push(json!({
                "step_id": item.get("step_id"),
                "dispatch_label": item.get("dispatch_label"),
                "attempt": item.get("attempt"),
                "overlay": overlay,
            }));
        }
    }
    Value::Array(overlays)
}
