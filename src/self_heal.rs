use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use chrono::Utc;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::runtime::Handle;
use tracing::warn;

#[derive(Debug, Error)]
pub enum SelfHealError {
    #[error("failed to read self-heal config: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse self-heal config: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("strategy '{0}' not found")]
    UnknownStrategy(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfHealAction {
    AutoRetry {
        extra_attempts: u8,
    },
    Annotate {
        severity: Option<String>,
        note: Option<String>,
    },
    HumanApproval {
        severity: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelfHealStrategy {
    pub id: String,
    pub description: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub telemetry_label: Option<String>,
    pub action: SelfHealAction,
}

#[derive(Clone, Debug, Serialize)]
pub struct SelfHealEvent {
    pub timestamp: i64,
    pub strategy_id: String,
    pub action: String,
    pub note: Option<String>,
}

static SELF_HEAL_EVENTS: Lazy<RwLock<Vec<SelfHealEvent>>> = Lazy::new(|| RwLock::new(Vec::new()));
static WEBHOOK_CLIENT: Lazy<Client> = Lazy::new(Client::new);

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
struct SelfHealConfigFile {
    strategies: Vec<SelfHealStrategy>,
}

#[derive(Debug)]
pub struct SelfHealRegistry {
    strategies: RwLock<HashMap<String, SelfHealStrategy>>,
    config_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct SelfHealRegistryStats {
    pub total_strategies: usize,
    pub enabled_strategies: usize,
}

impl SelfHealRegistry {
    pub fn load_from_path(path: Option<PathBuf>) -> Result<Self, SelfHealError> {
        let strategies = if let Some(ref path) = path {
            if path.exists() {
                let raw = fs::read_to_string(path)?;
                let parsed: SelfHealConfigFile = serde_yaml::from_str(&raw)?;
                parsed.strategies
            } else {
                default_strategies()
            }
        } else {
            default_strategies()
        };
        Ok(Self::new(path, strategies))
    }

    fn new(path: Option<PathBuf>, strategies: Vec<SelfHealStrategy>) -> Self {
        let map = strategies
            .into_iter()
            .map(|s| (s.id.clone(), s))
            .collect::<HashMap<_, _>>();
        Self {
            strategies: RwLock::new(map),
            config_path: path,
        }
    }

    pub fn strategies(&self) -> Vec<SelfHealStrategy> {
        self.strategies.read().values().cloned().collect()
    }

    pub fn strategy(&self, id: &str) -> Option<SelfHealStrategy> {
        self.strategies.read().get(id).cloned()
    }

    pub fn stats(&self) -> SelfHealRegistryStats {
        let guard = self.strategies.read();
        let total = guard.len();
        let enabled = guard.values().filter(|strategy| strategy.enabled).count();
        SelfHealRegistryStats {
            total_strategies: total,
            enabled_strategies: enabled,
        }
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), SelfHealError> {
        let mut guard = self.strategies.write();
        let strategy = guard
            .get_mut(id)
            .ok_or_else(|| SelfHealError::UnknownStrategy(id.to_string()))?;
        strategy.enabled = enabled;
        drop(guard);
        self.persist()?;
        record_event(SelfHealEvent {
            timestamp: Utc::now().timestamp_millis(),
            strategy_id: id.to_string(),
            action: if enabled { "enabled" } else { "disabled" }.to_string(),
            note: None,
        });
        Ok(())
    }

    pub fn auto_retry_extra_attempts(&self) -> u8 {
        self.strategy("auto_retry")
            .and_then(|s| {
                if !s.enabled {
                    return None;
                }
                match s.action {
                    SelfHealAction::AutoRetry { extra_attempts } => Some(extra_attempts),
                    _ => None,
                }
            })
            .unwrap_or(0)
    }

    pub fn enabled_strategy(&self, id: &str) -> Option<SelfHealStrategy> {
        self.strategy(id).filter(|s| s.enabled)
    }

    fn persist(&self) -> Result<(), SelfHealError> {
        let Some(path) = self.config_path.as_ref() else {
            return Ok(());
        };
        let snapshot = SelfHealConfigFile {
            strategies: self.strategies(),
        };
        let yaml = serde_yaml::to_string(&snapshot).map_err(SelfHealError::Parse)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, yaml)?;
        Ok(())
    }
}

pub fn record_event(event: SelfHealEvent) {
    let mut guard = SELF_HEAL_EVENTS.write();
    guard.push(event);
    let len = guard.len();
    if len > 200 {
        let remove_count = len.saturating_sub(200);
        guard.drain(0..remove_count);
    }
    let latest_event = guard.last().cloned();
    drop(guard);
    if let (Some(url), Some(event_payload)) = (webhook_url(), latest_event) {
        trigger_webhook(url, event_payload);
    }
}

fn webhook_url() -> Option<String> {
    match std::env::var("SOULBROWSER_SELF_HEAL_WEBHOOK") {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => None,
    }
}

fn trigger_webhook(url: String, event: SelfHealEvent) {
    if Handle::try_current().is_err() {
        warn!("self-heal webhook skipped (no async runtime)");
        return;
    }
    tokio::spawn(async move {
        let client = WEBHOOK_CLIENT.clone();
        if let Err(err) = client.post(&url).json(&event).send().await {
            warn!(target: "self_heal", ?err, "failed to deliver self-heal webhook");
        }
    });
}

impl Default for SelfHealRegistry {
    fn default() -> Self {
        Self::new(None, default_strategies())
    }
}

fn default_strategies() -> Vec<SelfHealStrategy> {
    vec![
        SelfHealStrategy {
            id: "auto_retry".to_string(),
            description: "Automatically retry failed dispatches".to_string(),
            enabled: true,
            tags: vec!["retry".into(), "stability".into()],
            telemetry_label: Some("auto_retry".into()),
            action: SelfHealAction::AutoRetry { extra_attempts: 1 },
        },
        SelfHealStrategy {
            id: "switch_to_baidu".to_string(),
            description: "Switch search intent execution to Baidu after Google blocks".to_string(),
            enabled: true,
            tags: vec!["fallback".into(), "search".into()],
            telemetry_label: Some("switch_to_baidu".into()),
            action: SelfHealAction::Annotate {
                severity: Some("info".into()),
                note: Some("Detected Google blocker; switching to Baidu".into()),
            },
        },
        SelfHealStrategy {
            id: "human_confirmation".to_string(),
            description: "Escalate terminal failures for human confirmation".to_string(),
            enabled: false,
            tags: vec!["escalation".into()],
            telemetry_label: Some("human_confirm".into()),
            action: SelfHealAction::HumanApproval {
                severity: Some("warn".into()),
            },
        },
        SelfHealStrategy {
            id: "require_manual_captcha".to_string(),
            description: "Pause automation to resolve CAPTCHA manually".to_string(),
            enabled: true,
            tags: vec!["captcha".into(), "escalation".into()],
            telemetry_label: Some("manual_captcha".into()),
            action: SelfHealAction::HumanApproval {
                severity: Some("warn".into()),
            },
        },
        SelfHealStrategy {
            id: "ack_permission_prompt".to_string(),
            description: "Handle browser permission prompts before continuing".to_string(),
            enabled: true,
            tags: vec!["fallback".into()],
            telemetry_label: Some("permission_prompt".into()),
            action: SelfHealAction::Annotate {
                severity: Some("info".into()),
                note: Some(
                    "Permission prompt detected; ensure notifications/media access is granted."
                        .into(),
                ),
            },
        },
        SelfHealStrategy {
            id: "wait_download_complete".to_string(),
            description: "Wait for downloads before proceeding".to_string(),
            enabled: true,
            tags: vec!["fallback".into()],
            telemetry_label: Some("download_wait".into()),
            action: SelfHealAction::Annotate {
                severity: Some("info".into()),
                note: Some("Download in progress; wait or switch approach.".into()),
            },
        },
    ]
}

pub fn default_config_path(storage_path: Option<&Path>) -> PathBuf {
    if let Some(root) = storage_path {
        let mut path = root.to_path_buf();
        path.push("self_heal.yaml");
        path
    } else {
        PathBuf::from("config/self_heal.yaml")
    }
}
