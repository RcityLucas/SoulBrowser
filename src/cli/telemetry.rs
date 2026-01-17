use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use soulbrowser_kernel::telemetry::{self, register_sink, TelemetryEvent, TelemetrySink};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use tokio::signal;
use uuid::Uuid;

const MAX_SEND_RETRIES: usize = 3;
const RETRY_BASE_DELAY_MS: u64 = 250;

use crate::cli::context::CliContext;

#[derive(Args, Clone, Debug)]
pub struct TelemetryArgs {
    #[command(subcommand)]
    pub command: TelemetryCommand,
}

#[derive(Subcommand, Clone, Debug)]
pub enum TelemetryCommand {
    /// Stream telemetry events emitted during execution (Ctrl+C to stop)
    Tail,
    /// Configure telemetry sinks from CLI (stdout already controlled via env)
    Webhook(WebhookArgs),
    /// Send telemetry events to PostHog
    Posthog(PosthogArgs),
    /// List persisted telemetry sinks
    List,
    /// Remove a persisted sink by kind/id
    Remove(RemoveArgs),
}

#[derive(Args, Clone, Debug)]
pub struct WebhookArgs {
    /// Destination endpoint (HTTP POST)
    #[arg(long)]
    pub url: String,
    /// Optional bearer token
    #[arg(long)]
    pub bearer: Option<String>,
}

#[derive(Args, Clone, Debug)]
pub struct PosthogArgs {
    /// PostHog API host (default https://app.posthog.com)
    #[arg(long, default_value = "https://app.posthog.com")]
    pub host: String,
    /// Project API key
    #[arg(long)]
    pub api_key: String,
    /// Distinct id for analytics (defaults to tenant `cli`)
    #[arg(long)]
    pub distinct_id: Option<String>,
}

pub async fn cmd_telemetry(args: TelemetryArgs, ctx: &CliContext) -> Result<()> {
    match args.command {
        TelemetryCommand::Tail => tail_events().await,
        TelemetryCommand::Webhook(args) => configure_webhook(args, ctx).await,
        TelemetryCommand::Posthog(args) => configure_posthog(args, ctx).await,
        TelemetryCommand::List => list_sinks(ctx),
        TelemetryCommand::Remove(args) => remove_sink(args, ctx),
    }
}

async fn tail_events() -> Result<()> {
    let mut receiver = telemetry::subscribe();
    println!("Listening for telemetry events... (Ctrl+C to stop)");
    loop {
        tokio::select! {
            event = receiver.recv() => {
                match event {
                    Ok(entry) => {
                        println!("{}", serde_json::to_string(&entry)?);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        println!("[telemetry] dropped {} events", skipped);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = signal::ctrl_c() => {
                println!("\nTelemetry tail stopped by user");
                break;
            }
        }
    }
    Ok(())
}

async fn configure_webhook(args: WebhookArgs, ctx: &CliContext) -> Result<()> {
    let path = telemetry_config_path(ctx);
    let mut cfg = load_config(&path)?;
    ensure_device_id(&mut cfg);
    let entry = WebhookEntry {
        url: args.url,
        bearer: args.bearer,
    };
    upsert_webhook(&mut cfg, entry.clone());
    save_config(&path, &cfg)?;
    register_sink(build_webhook_sink(&entry)?);
    println!("Webhook telemetry sink registered");
    Ok(())
}

struct WebhookSink {
    client: Client,
    endpoint: String,
    bearer: Option<String>,
}

impl TelemetrySink for WebhookSink {
    fn emit(&self, event: &TelemetryEvent) {
        for attempt in 0..MAX_SEND_RETRIES {
            let mut request = self.client.post(&self.endpoint).json(event);
            if let Some(token) = &self.bearer {
                request = request.bearer_auth(token);
            }
            match request.send() {
                Ok(_) => return,
                Err(err) => {
                    if attempt + 1 >= MAX_SEND_RETRIES {
                        eprintln!("[telemetry] webhook send failed after retries: {err}");
                    } else {
                        let delay = RETRY_BASE_DELAY_MS * (attempt as u64 + 1);
                        thread::sleep(Duration::from_millis(delay));
                    }
                }
            }
        }
    }
}

async fn configure_posthog(args: PosthogArgs, ctx: &CliContext) -> Result<()> {
    let path = telemetry_config_path(ctx);
    let mut cfg = load_config(&path)?;
    let device_id = ensure_device_id(&mut cfg);
    let entry = PosthogEntry {
        host: args.host,
        api_key: args.api_key,
        distinct_id: args.distinct_id.or(device_id.clone()),
    };
    cfg.posthog = Some(entry.clone());
    save_config(&path, &cfg)?;
    register_sink(build_posthog_sink(&entry, device_id.as_deref())?);
    println!("PostHog telemetry sink registered");
    Ok(())
}

struct PosthogSink {
    client: Client,
    host: String,
    api_key: String,
    distinct_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TelemetryConfigFile {
    device_id: Option<String>,
    webhooks: Vec<WebhookEntry>,
    posthog: Option<PosthogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WebhookEntry {
    url: String,
    bearer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PosthogEntry {
    host: String,
    api_key: String,
    distinct_id: Option<String>,
}

pub fn load_persistent_sinks(config_dir: &Path) -> Result<()> {
    let path = config_dir.join("telemetry.json");
    let mut cfg = load_config(&path)?;
    let device_id = ensure_device_id(&mut cfg);
    if path.exists() {
        save_config(&path, &cfg)?;
    } else if cfg.device_id.is_some() {
        save_config(&path, &cfg)?;
    }
    for entry in cfg.webhooks.iter() {
        register_sink(build_webhook_sink(entry)?);
    }
    if let Some(entry) = cfg.posthog.as_ref() {
        register_sink(build_posthog_sink(entry, device_id.as_deref())?);
    }
    Ok(())
}

fn telemetry_config_path(ctx: &CliContext) -> PathBuf {
    ctx.config_dir().join("telemetry.json")
}

fn load_config(path: &Path) -> Result<TelemetryConfigFile> {
    if path.exists() {
        let raw = fs::read(path)?;
        Ok(serde_json::from_slice(&raw)?)
    } else {
        Ok(TelemetryConfigFile::default())
    }
}

fn save_config(path: &Path, cfg: &TelemetryConfigFile) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let payload = serde_json::to_vec_pretty(cfg)?;
    fs::write(path, payload)?;
    Ok(())
}

fn ensure_device_id(cfg: &mut TelemetryConfigFile) -> Option<String> {
    if cfg.device_id.is_none() {
        cfg.device_id = Some(Uuid::new_v4().to_string());
    }
    cfg.device_id.clone()
}

fn upsert_webhook(cfg: &mut TelemetryConfigFile, entry: WebhookEntry) {
    cfg.webhooks.retain(|existing| existing.url != entry.url);
    cfg.webhooks.push(entry);
}

fn build_webhook_sink(entry: &WebhookEntry) -> Result<Box<dyn TelemetrySink>> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|err| anyhow!("failed to configure HTTP client: {err}"))?;
    Ok(Box::new(WebhookSink {
        client,
        endpoint: entry.url.clone(),
        bearer: entry.bearer.clone(),
    }))
}

fn build_posthog_sink(
    entry: &PosthogEntry,
    default_distinct: Option<&str>,
) -> Result<Box<dyn TelemetrySink>> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|err| anyhow!("failed to configure HTTP client: {err}"))?;
    let distinct = entry
        .distinct_id
        .clone()
        .or_else(|| default_distinct.map(|s| s.to_string()))
        .unwrap_or_else(|| "cli".to_string());
    Ok(Box::new(PosthogSink {
        client,
        host: entry.host.clone(),
        api_key: entry.api_key.clone(),
        distinct_id: distinct,
    }))
}

impl TelemetrySink for PosthogSink {
    fn emit(&self, event: &TelemetryEvent) {
        let endpoint = format!("{}/capture", self.host.trim_end_matches('/'));
        let payload = serde_json::json!({
            "event": format!("telemetry.{}", match event.kind {
                soulbrowser_kernel::telemetry::TelemetryEventKind::StepCompleted => "step_completed",
                soulbrowser_kernel::telemetry::TelemetryEventKind::StepFailed => "step_failed",
            }),
            "distinct_id": self.distinct_id,
            "timestamp": event.timestamp,
            "properties": {
                "tenant": event.tenant,
                "task_id": event.task_id,
                "payload": event.payload,
                "metrics": {
                    "llm_input_tokens": event.metrics.llm_input_tokens,
                    "llm_output_tokens": event.metrics.llm_output_tokens,
                    "runtime_ms": event.metrics.runtime_ms,
                }
            }
        });
        for attempt in 0..MAX_SEND_RETRIES {
            let result = self
                .client
                .post(&endpoint)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&payload)
                .send();
            match result {
                Ok(_) => return,
                Err(err) => {
                    if attempt + 1 >= MAX_SEND_RETRIES {
                        eprintln!("[telemetry] PostHog send failed after retries: {err}");
                    } else {
                        let delay = RETRY_BASE_DELAY_MS * (attempt as u64 + 1);
                        thread::sleep(Duration::from_millis(delay));
                    }
                }
            }
        }
    }
}
#[derive(Args, Clone, Debug)]
pub struct RemoveArgs {
    /// Sink kind (webhook|posthog)
    #[arg(long, value_parser = ["webhook", "posthog"], value_name = "KIND")]
    pub kind: String,
    /// Identifier (URL for webhook)
    #[arg(long)]
    pub id: Option<String>,
}
fn list_sinks(ctx: &CliContext) -> Result<()> {
    let cfg = load_config(&telemetry_config_path(ctx))?;
    println!(
        "Device ID: {}",
        cfg.device_id.as_deref().unwrap_or("<none>")
    );
    if cfg.webhooks.is_empty() {
        println!("Webhooks: [none]");
    } else {
        println!("Webhooks:");
        for entry in &cfg.webhooks {
            println!("  - {}", entry.url);
        }
    }
    if let Some(entry) = cfg.posthog {
        println!(
            "PostHog: {} (distinct={})",
            entry.host,
            entry.distinct_id.unwrap_or_else(|| "<tenant>".into())
        );
    } else {
        println!("PostHog: [none]");
    }
    Ok(())
}

fn remove_sink(args: RemoveArgs, ctx: &CliContext) -> Result<()> {
    let path = telemetry_config_path(ctx);
    let mut cfg = load_config(&path)?;
    match args.kind.as_str() {
        "webhook" => {
            let Some(id) = args.id.as_deref() else {
                return Err(anyhow!("--id must be provided for webhook"));
            };
            let before = cfg.webhooks.len();
            cfg.webhooks.retain(|entry| entry.url != id);
            if cfg.webhooks.len() == before {
                return Err(anyhow!("Webhook {} not found", id));
            }
            save_config(&path, &cfg)?;
            println!("Removed webhook {}", id);
        }
        "posthog" => {
            cfg.posthog = None;
            save_config(&path, &cfg)?;
            println!("Removed PostHog sink");
        }
        _ => unreachable!(),
    }
    Ok(())
}
