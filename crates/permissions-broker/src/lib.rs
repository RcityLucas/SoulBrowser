pub mod config;

pub use crate::config::{Needs, PolicyFile, PolicyTemplate, SitePolicy};

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Identifier for policies stored in the broker.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct PolicyId(pub Uuid);

/// High-level decision returned to callers after applying a policy or ensuring permissions.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthzDecision {
    pub kind: DecisionKind,
    pub allowed: Vec<String>,
    pub denied: Vec<String>,
    pub missing: Vec<String>,
    pub ttl_ms: Option<u64>,
}

/// Decision outcome categories.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DecisionKind {
    Allow = 0,
    Deny = 1,
    Partial = 2,
}

/// Errors produced by the broker surface.
#[derive(Clone, Debug, Error)]
pub enum BrokerError {
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    #[error("cdp I/O failure: {0}")]
    CdpIo(String),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Default)]
struct PolicyStore {
    file: Option<PolicyFile>,
}

impl PolicyStore {
    fn update(&mut self, file: PolicyFile) {
        self.file = Some(file);
    }

    fn resolve(&self, origin: &str) -> Option<(PolicyTemplate, Option<Duration>)> {
        let file = self.file.as_ref()?;
        let mut template = file.defaults.clone();
        let mut ttl = parse_ttl(template.ttl.as_deref()).unwrap_or(None);
        let mut best_match_len = 0usize;

        for site in &file.sites {
            if pattern_matches(&site.match_pattern, origin) {
                let match_len = site.match_pattern.len();
                if match_len >= best_match_len {
                    best_match_len = match_len;
                    if let Some(allow) = &site.allow {
                        template.allow = allow.clone();
                    }
                    if let Some(deny) = &site.deny {
                        template.deny = deny.clone();
                    }
                    if let Some(site_ttl) = &site.ttl {
                        if let Ok(parsed) = parse_ttl(Some(site_ttl)) {
                            ttl = parsed;
                        }
                        template.ttl = Some(site_ttl.clone());
                    }
                }
            }
        }

        Some((template, ttl))
    }
}

fn pattern_matches(pattern: &str, origin: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.len() == 2 {
            let prefix = parts[0];
            let suffix = parts[1];
            return origin.starts_with(prefix) && origin.ends_with(suffix);
        }
    }
    origin == pattern
}

fn parse_ttl(raw: Option<&str>) -> Result<Option<Duration>, BrokerError> {
    let Some(ttl_str) = raw else {
        return Ok(None);
    };

    if ttl_str.eq_ignore_ascii_case("session") {
        return Ok(None);
    }

    let duration = humantime::parse_duration(ttl_str)
        .map_err(|_| BrokerError::Internal(format!("invalid ttl format: {ttl_str}")))?;
    Ok(Some(duration))
}

/// Broker state with an in-memory policy store.
pub struct PermissionsBroker {
    store: RwLock<PolicyStore>,
}

impl PermissionsBroker {
    pub fn new() -> Self {
        Self {
            store: RwLock::new(PolicyStore::default()),
        }
    }

    pub async fn load_policy(&self, policy: PolicyFile) {
        let mut guard = self.store.write().await;
        guard.update(policy);
    }
}

/// Trait describing the operations exposed to higher layers.
#[async_trait]
pub trait Broker {
    async fn apply_policy(&self, origin: &str) -> Result<AuthzDecision, BrokerError>;
    async fn ensure_for(
        &self,
        origin: &str,
        needs: &[String],
    ) -> Result<AuthzDecision, BrokerError>;
    async fn revoke(&self, origin: &str, which: Option<Vec<String>>) -> Result<(), BrokerError>;
}

#[async_trait]
impl Broker for PermissionsBroker {
    async fn apply_policy(&self, origin: &str) -> Result<AuthzDecision, BrokerError> {
        let guard = self.store.read().await;
        let (template, ttl) = guard
            .resolve(origin)
            .ok_or_else(|| BrokerError::PolicyDenied(format!("no policy for {origin}")))?;
        Ok(decision_from_template(template, None, ttl))
    }

    async fn ensure_for(
        &self,
        origin: &str,
        needs: &[String],
    ) -> Result<AuthzDecision, BrokerError> {
        let guard = self.store.read().await;
        let (template, ttl) = guard
            .resolve(origin)
            .ok_or_else(|| BrokerError::PolicyDenied(format!("no policy for {origin}")))?;
        Ok(decision_from_template(template, Some(needs), ttl))
    }

    async fn revoke(&self, origin: &str, _which: Option<Vec<String>>) -> Result<(), BrokerError> {
        Err(BrokerError::Internal(format!(
            "revoke not implemented for {origin}"
        )))
    }
}

fn decision_from_template(
    template: PolicyTemplate,
    needs: Option<&[String]>,
    ttl: Option<Duration>,
) -> AuthzDecision {
    let mut allowed = template.allow;
    let denied = template.deny;
    if let Some(req) = needs {
        allowed.retain(|perm| req.contains(perm));
    }
    let missing = needs
        .map(|req| {
            req.iter()
                .filter(|perm| !allowed.contains(perm))
                .cloned()
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    let requested_len = needs.map(|n| n.len()).unwrap_or(0);

    let kind = if missing.is_empty() && denied.is_empty() {
        DecisionKind::Allow
    } else if !missing.is_empty() && requested_len > 0 && missing.len() == requested_len {
        DecisionKind::Deny
    } else {
        DecisionKind::Partial
    };

    AuthzDecision {
        kind,
        allowed,
        denied,
        missing,
        ttl_ms: ttl.map(|d| d.as_millis() as u64),
    }
}
