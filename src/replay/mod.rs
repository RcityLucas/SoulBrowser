//! Session replay module
#![allow(dead_code)]
//!
//! Provides functionality to replay recorded browser sessions

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::{
    app_context::AppContext,
    browser_impl::{BrowserConfig, L0Protocol, L1BrowserManager},
    storage::{QueryParams, StorageManager},
    types::BrowserType,
};

/// Session replayer for replaying recorded browser sessions
pub struct SessionReplayer {
    config: ReplayConfig,
    storage_manager: Arc<StorageManager>,
}

/// Configuration for session replay
#[derive(Debug, Clone)]
pub struct ReplayConfig {
    pub recording_path: PathBuf,
    pub browser_type: BrowserType,
    pub playback_speed: f64,
    pub headless: bool,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            recording_path: PathBuf::from("./recordings"),
            browser_type: BrowserType::Chromium,
            playback_speed: 1.0,
            headless: false,
        }
    }
}

/// Results from replay execution
#[derive(Debug, Serialize)]
pub struct ReplayResults {
    pub success: bool,
    pub events_replayed: usize,
    pub duration: u64,
    pub errors: Vec<String>,
}

impl SessionReplayer {
    /// Create a new session replayer with app context
    pub fn with_context(context: Arc<AppContext>, config: ReplayConfig) -> Self {
        Self {
            config,
            storage_manager: context.storage(),
        }
    }

    /// Replay a recorded session
    pub async fn replay_session(
        &self,
        session_id: &str,
        overrides: Option<&HashMap<String, String>>,
        fail_fast: bool,
    ) -> Result<ReplayResults> {
        let start = std::time::Instant::now();
        let mut errors = Vec::new();
        let mut events_replayed = 0;

        // Query events for the session
        let query = QueryParams {
            session_id: Some(session_id.to_string()),
            event_type: None,
            from_timestamp: None,
            to_timestamp: None,
            limit: 1000,
            offset: 0,
        };

        let events = self
            .storage_manager
            .backend()
            .query_events(query)
            .await
            .context("Failed to query session events")?;

        if events.is_empty() {
            return Err(anyhow::anyhow!(
                "No events found for session {}",
                session_id
            ));
        }

        // Initialize browser for replay
        let l0 = L0Protocol::new()
            .await
            .context("Failed to initialize L0 protocol")?;

        let browser_config = BrowserConfig {
            browser_type: self.config.browser_type.clone(),
            headless: self.config.headless,
            window_size: Some((1280, 720)),
            devtools: false,
        };

        let mut browser_manager = L1BrowserManager::new(l0, browser_config)
            .await
            .context("Failed to initialize browser manager")?;

        let browser = browser_manager
            .launch_browser()
            .await
            .context("Failed to launch browser")?;

        let mut page = browser.new_page().await.context("Failed to create page")?;

        // Replay each event
        let mut last_timestamp = events[0].timestamp;

        for event in events {
            // Calculate delay based on playback speed
            let delay = if event.timestamp > last_timestamp {
                let original_delay = (event.timestamp - last_timestamp) as f64;
                (original_delay / self.config.playback_speed) as u64
            } else {
                0
            };

            if delay > 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
            }

            // Replay the event based on its type
            match event.event_type.as_str() {
                "navigate" => {
                    if let Some(url) = event.data.get("url").and_then(|v| v.as_str()) {
                        let url = resolve_template(url, overrides);
                        if let Err(e) = page.navigate(&url).await {
                            errors.push(format!("Navigate failed: {}", e));
                            if fail_fast {
                                break;
                            }
                        } else {
                            events_replayed += 1;
                        }
                    }
                }
                "click" => {
                    if let Some(selector) = event.data.get("selector").and_then(|v| v.as_str()) {
                        let selector = resolve_template(selector, overrides);
                        if let Err(e) = page.click(&selector).await {
                            errors.push(format!("Click failed: {}", e));
                            if fail_fast {
                                break;
                            }
                        } else {
                            events_replayed += 1;
                        }
                    }
                }
                "type" => {
                    if let (Some(selector), Some(text)) = (
                        event.data.get("selector").and_then(|v| v.as_str()),
                        event.data.get("text").and_then(|v| v.as_str()),
                    ) {
                        let selector = resolve_template(selector, overrides);
                        let text = resolve_template(text, overrides);
                        if let Err(e) = page.type_text(&selector, &text).await {
                            errors.push(format!("Type failed: {}", e));
                            if fail_fast {
                                break;
                            }
                        } else {
                            events_replayed += 1;
                        }
                    }
                }
                "screenshot" => {
                    if let Some(filename) = event.data.get("filename").and_then(|v| v.as_str()) {
                        let filename = resolve_template(filename, overrides);
                        if let Err(e) = page.screenshot(&filename).await {
                            errors.push(format!("Screenshot failed: {}", e));
                            if fail_fast {
                                break;
                            }
                        } else {
                            events_replayed += 1;
                        }
                    }
                }
                _ => {
                    println!("Skipping unknown event type: {}", event.event_type);
                }
            }

            last_timestamp = event.timestamp;
            if fail_fast && !errors.is_empty() {
                break;
            }
        }

        Ok(ReplayResults {
            success: errors.is_empty(),
            events_replayed,
            duration: start.elapsed().as_secs(),
            errors,
        })
    }

    /// List available sessions for replay
    pub async fn list_sessions(&self) -> Result<Vec<String>> {
        // Query unique session IDs
        let sessions = self
            .storage_manager
            .backend()
            .list_sessions()
            .await
            .context("Failed to list sessions")?;

        Ok(sessions.into_iter().map(|s| s.id).collect())
    }
}

fn resolve_template(value: &str, overrides: Option<&HashMap<String, String>>) -> String {
    if let Some(map) = overrides {
        let mut result = value.to_string();
        for (key, replacement) in map {
            let token = format!("{{{{{}}}}}", key);
            if result.contains(&token) {
                result = result.replace(&token, replacement);
            }
        }
        result
    } else {
        value.to_string()
    }
}
