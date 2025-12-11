use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Args;
use serde_json::json;
use tracing::info;
use uuid::Uuid;

use crate::app_context::get_or_create_context;
use crate::browser_impl::{BrowserConfig, L0Protocol, L1BrowserManager};
use crate::storage::BrowserSessionEntity;
use crate::types::BrowserType;
use crate::{persist_event, Config};
use soulbase_types::tenant::TenantId;

#[derive(Args, Clone, Debug)]
pub struct RecordArgs {
    /// Session name
    pub name: String,

    /// Browser type to use
    #[arg(short, long, default_value = "chromium")]
    pub browser: BrowserType,

    /// Start URL
    #[arg(short, long)]
    pub url: Option<String>,

    /// Recording output directory
    #[arg(short, long)]
    pub output_dir: Option<PathBuf>,

    /// Enable screenshot recording
    #[arg(long)]
    pub screenshots: bool,

    /// Enable video recording
    #[arg(long)]
    pub video: bool,

    /// Record network activity
    #[arg(long)]
    pub network: bool,

    /// Record performance metrics
    #[arg(long)]
    pub performance: bool,
}

pub async fn cmd_record(args: RecordArgs, config: &Config) -> Result<()> {
    info!("Starting recording session: {}", args.name);

    let storage_path = args
        .output_dir
        .clone()
        .or_else(|| Some(config.output_dir.clone()));

    let context =
        get_or_create_context("cli".to_string(), storage_path, config.policy_paths.clone()).await?;
    let storage = context.storage();

    let tenant_id = TenantId("cli".to_string());
    let session_id = format!("record-{}-{}", args.name, Uuid::new_v4());
    let start_url = args.url.clone();

    info!(session_id, "Recording session initialized");

    let created_at = Utc::now().timestamp_millis();
    let session_entity = BrowserSessionEntity {
        id: session_id.clone(),
        tenant: tenant_id.clone(),
        subject_id: "recorder".to_string(),
        created_at,
        updated_at: created_at,
        state: "recording".to_string(),
        metadata: json!({
            "name": args.name,
            "url": start_url.clone(),
            "options": {
                "screenshots": args.screenshots,
                "video": args.video,
                "network": args.network,
                "performance": args.performance,
            }
        }),
    };

    storage
        .backend()
        .store_session(session_entity)
        .await
        .context("Failed to persist session metadata")?;

    let mut sequence: u64 = 1;
    persist_event(
        &storage,
        &tenant_id,
        &session_id,
        sequence,
        "recording_started",
        json!({
            "name": args.name,
            "url": start_url.clone(),
            "options": {
                "screenshots": args.screenshots,
                "video": args.video,
                "network": args.network,
                "performance": args.performance,
            }
        }),
    )
    .await?;
    sequence += 1;

    let l0 = L0Protocol::new().await?;
    let browser_config = BrowserConfig {
        browser_type: args.browser,
        headless: false,
        window_size: Some((1280, 720)),
        devtools: true,
    };
    let mut browser_manager = L1BrowserManager::new(l0, browser_config).await?;
    let browser = browser_manager.launch_browser().await?;
    let mut page = browser.new_page().await?;
    if let Some(url) = start_url.as_deref() {
        page.navigate(url).await?;
    }

    info!("Recording started. Interact with the browser and press Ctrl+C to stop.");
    tokio::signal::ctrl_c().await?;

    persist_event(
        &storage,
        &tenant_id,
        &session_id,
        sequence,
        "recording_stopped",
        json!({ "reason": "user_exit" }),
    )
    .await?;

    let updated_at = Utc::now().timestamp_millis();
    let completed_session = BrowserSessionEntity {
        id: session_id.clone(),
        tenant: tenant_id.clone(),
        subject_id: "recorder".to_string(),
        created_at,
        updated_at,
        state: "completed".to_string(),
        metadata: json!({
            "name": args.name,
            "url": start_url,
            "options": {
                "screenshots": args.screenshots,
                "video": args.video,
                "network": args.network,
                "performance": args.performance,
            }
        }),
    };

    storage
        .backend()
        .update_session(completed_session)
        .await
        .context("Failed to update session state")?;

    info!(
        "Recording session {} complete. Replay with: cargo run -- replay {}",
        session_id, session_id
    );

    Ok(())
}
