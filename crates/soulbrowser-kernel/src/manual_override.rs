use std::env;
use std::time::Duration;

use cdp_adapter::DebuggerEndpoint;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use dashmap::DashMap;
use once_cell::sync::OnceCell;
use prometheus::{IntGauge, Opts};
use serde::{Deserialize, Serialize};
use soulbrowser_core_types::TaskId;
use thiserror::Error;
use tracing::{info, warn};
use uuid::Uuid;

use crate::metrics;

#[derive(Clone, Debug)]
pub struct ManualOverrideConfig {
    pub enabled: bool,
    pub timeout: Duration,
}

impl ManualOverrideConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: parse_bool_env("SOUL_MANUAL_TAKEOVER_ENABLED"),
            timeout: resolve_timeout(),
        }
    }
}

fn parse_bool_env(var: &str) -> bool {
    match env::var(var) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

fn resolve_timeout() -> Duration {
    let default_secs = 300;
    match env::var("SOUL_MANUAL_TAKEOVER_TIMEOUT_SECS") {
        Ok(value) => value
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|secs| *secs > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(default_secs)),
        Err(_) => Duration::from_secs(default_secs),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManualRouteContext {
    pub session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ManualTakeoverRequest {
    pub task_id: TaskId,
    pub debugger: DebuggerEndpoint,
    pub route: ManualRouteContext,
    pub requested_by: Option<String>,
    pub expires_in: Option<Duration>,
}

#[derive(Clone, Debug)]
pub struct ManualTakeoverResponse {
    pub resume_token: String,
    pub snapshot: ManualOverrideSnapshot,
}

#[derive(Debug, Error)]
pub enum ManualOverrideError {
    #[error("manual takeover disabled")]
    Disabled,
    #[error("manual takeover already active")]
    AlreadyActive,
    #[error("manual override not found")]
    NotFound,
    #[error("resume token mismatch")]
    InvalidToken,
    #[error("manual override expired")]
    Expired,
}

#[derive(Clone)]
struct ManualOverrideRecord {
    task_id: TaskId,
    status: ManualOverridePhase,
    requested_at: DateTime<Utc>,
    activated_at: Option<DateTime<Utc>>,
    resumed_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
    resume_token: String,
    debugger: DebuggerEndpoint,
    requested_by: Option<String>,
    route: ManualRouteContext,
}

impl ManualOverrideRecord {
    fn snapshot(&self) -> ManualOverrideSnapshot {
        ManualOverrideSnapshot {
            task_id: self.task_id.clone(),
            status: self.status,
            requested_at: self.requested_at,
            activated_at: self.activated_at,
            resumed_at: self.resumed_at,
            expires_at: self.expires_at,
            requested_by: self.requested_by.clone(),
            debugger: Some(self.debugger.clone()),
            route: Some(self.route.clone()),
        }
    }
}

pub struct ManualSessionManager {
    config: ManualOverrideConfig,
    sessions: DashMap<String, ManualOverrideRecord>,
}

impl ManualSessionManager {
    pub fn new(config: ManualOverrideConfig) -> Self {
        Self {
            config,
            sessions: DashMap::new(),
        }
    }

    pub fn from_env() -> Self {
        Self::new(ManualOverrideConfig::from_env())
    }

    pub fn config(&self) -> &ManualOverrideConfig {
        &self.config
    }

    pub fn request_takeover(
        &self,
        request: ManualTakeoverRequest,
    ) -> Result<ManualTakeoverResponse, ManualOverrideError> {
        if !self.config.enabled {
            return Err(ManualOverrideError::Disabled);
        }
        if self.sessions.contains_key(&request.task_id.0) {
            return Err(ManualOverrideError::AlreadyActive);
        }

        let ManualTakeoverRequest {
            task_id,
            debugger,
            route,
            requested_by,
            expires_in,
        } = request;

        let resume_token = Uuid::new_v4().to_string();
        let requested_at = Utc::now();
        let expires_at = requested_at + self.resolve_deadline(expires_in);
        let record = ManualOverrideRecord {
            task_id: task_id.clone(),
            status: ManualOverridePhase::Requested,
            requested_at,
            activated_at: None,
            resumed_at: None,
            expires_at,
            resume_token: resume_token.clone(),
            debugger,
            requested_by,
            route,
        };
        let snapshot = record.snapshot();
        self.sessions.insert(task_id.0.clone(), record);
        self.update_metric();
        info!(task = %task_id.0, expires_at = %snapshot.expires_at, "manual takeover requested");
        Ok(ManualTakeoverResponse {
            resume_token,
            snapshot,
        })
    }

    pub fn snapshot(&self, task_id: &TaskId) -> Option<ManualOverrideSnapshot> {
        self.sessions.get(&task_id.0).map(|entry| entry.snapshot())
    }

    pub fn set_phase(
        &self,
        task_id: &TaskId,
        phase: ManualOverridePhase,
    ) -> Option<ManualOverrideSnapshot> {
        let mut entry = self.sessions.get_mut(&task_id.0)?;
        let now = Utc::now();
        if matches!(phase, ManualOverridePhase::Active) && entry.activated_at.is_none() {
            entry.activated_at = Some(now);
        }
        if matches!(phase, ManualOverridePhase::Resuming) {
            entry.resumed_at = Some(now);
        }
        entry.status = phase;
        Some(entry.snapshot())
    }

    pub fn resume(
        &self,
        task_id: &TaskId,
        token: &str,
    ) -> Result<ManualOverrideSnapshot, ManualOverrideError> {
        let Some((_, mut entry)) = self.sessions.remove(&task_id.0) else {
            return Err(ManualOverrideError::NotFound);
        };
        if entry.resume_token != token {
            // put entry back to avoid losing state
            self.sessions.insert(task_id.0.clone(), entry);
            self.update_metric();
            return Err(ManualOverrideError::InvalidToken);
        }
        if entry.expires_at < Utc::now() {
            self.update_metric();
            return Err(ManualOverrideError::Expired);
        }
        entry.status = ManualOverridePhase::Resuming;
        entry.resumed_at = Some(Utc::now());
        let snapshot = entry.snapshot();
        self.update_metric();
        Ok(snapshot)
    }

    pub fn cancel(&self, task_id: &TaskId) -> bool {
        let removed = self.sessions.remove(&task_id.0).is_some();
        if removed {
            self.update_metric();
        }
        removed
    }

    pub fn gc(&self) -> usize {
        let now = Utc::now();
        let mut removed = 0usize;
        self.sessions.retain(|_, record| {
            if record.expires_at <= now {
                removed += 1;
                false
            } else {
                true
            }
        });
        if removed > 0 {
            self.update_metric();
        }
        removed
    }

    fn resolve_deadline(&self, override_ttl: Option<Duration>) -> ChronoDuration {
        let ttl = override_ttl.unwrap_or(self.config.timeout);
        ChronoDuration::from_std(ttl).unwrap_or_else(|_| ChronoDuration::seconds(300))
    }

    fn update_metric(&self) {
        let gauge = manual_override_gauge();
        gauge.set(self.sessions.len() as i64);
    }
}

fn manual_override_gauge() -> &'static IntGauge {
    static GAUGE: OnceCell<IntGauge> = OnceCell::new();
    GAUGE.get_or_init(|| {
        metrics::register_metrics();
        let gauge = IntGauge::with_opts(Opts::new(
            "soul_manual_override_active",
            "Active manual takeover sessions",
        ))
        .expect("manual override gauge");
        if let Err(err) = metrics::global_registry().register(Box::new(gauge.clone())) {
            warn!(?err, "failed to register manual override gauge");
        }
        gauge
    })
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManualOverridePhase {
    Requested,
    Active,
    Resuming,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManualOverrideSnapshot {
    pub task_id: TaskId,
    pub status: ManualOverridePhase,
    pub requested_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activated_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumed_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debugger: Option<DebuggerEndpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<ManualRouteContext>,
}
