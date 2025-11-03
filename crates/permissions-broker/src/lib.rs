mod cdp_transport;
pub mod config;

pub use crate::config::{
    ConfigError, Needs, PermissionMap, PolicyFile, PolicyTemplate, SitePolicy,
};
pub use cdp_transport::CdpPermissionTransport;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use cdp_adapter::Cdp;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{broadcast, RwLock};
use tracing::warn;
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

/// Event emitted whenever the broker issues a decision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEvent {
    pub origin: String,
    pub decision: DecisionKind,
    pub allowed: Vec<String>,
    pub denied: Vec<String>,
    pub missing: Vec<String>,
    pub ttl_ms: Option<u64>,
    pub timestamp: SystemTime,
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

    fn resolve(&self, origin: &str) -> Option<ResolvedPolicy> {
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

        Some(ResolvedPolicy { template, ttl })
    }
}

#[derive(Clone)]
struct ResolvedPolicy {
    template: PolicyTemplate,
    ttl: Option<Duration>,
}

#[derive(Clone)]
struct CachedPolicy {
    template: PolicyTemplate,
    ttl: Option<Duration>,
    expires_at: Option<Instant>,
}

impl CachedPolicy {
    fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(deadline) => Instant::now() >= deadline,
            None => false,
        }
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
    cache: DashMap<String, CachedPolicy>,
    events: broadcast::Sender<AuditEvent>,
    transport: RwLock<Option<Arc<dyn PermissionTransport>>>,
    permission_map: RwLock<Option<PermissionMap>>,
}

impl PermissionsBroker {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(128);
        Self {
            store: RwLock::new(PolicyStore::default()),
            cache: DashMap::new(),
            events: tx,
            transport: RwLock::new(None),
            permission_map: RwLock::new(None),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AuditEvent> {
        self.events.subscribe()
    }

    pub async fn load_policy(&self, policy: PolicyFile) -> Result<(), BrokerError> {
        self.validate_policy(&policy).await?;
        {
            let mut guard = self.store.write().await;
            guard.update(policy);
        }
        self.cache.clear();
        Ok(())
    }

    pub async fn set_permission_map(&self, map: PermissionMap) {
        let mut guard = self.permission_map.write().await;
        *guard = Some(map);
    }

    pub async fn set_transport(&self, transport: Arc<dyn PermissionTransport>) {
        let mut guard = self.transport.write().await;
        *guard = Some(transport);
    }

    /// Attach a live CDP adapter so broker decisions can update browser permissions.
    pub async fn attach_cdp_adapter(&self, adapter: Arc<dyn Cdp + Send + Sync>) {
        let transport = Arc::new(CdpPermissionTransport::new(adapter));
        self.set_transport(transport).await;
    }

    async fn validate_policy(&self, policy: &PolicyFile) -> Result<(), BrokerError> {
        let guard = self.permission_map.read().await;
        let Some(permission_map) = guard.as_ref() else {
            return Ok(());
        };

        let mut invalid = HashSet::new();

        for name in policy.defaults.allow.iter().chain(&policy.defaults.deny) {
            if !permission_map.contains_key(name) {
                invalid.insert(name.clone());
            }
        }

        for site in &policy.sites {
            if let Some(allow) = &site.allow {
                for name in allow {
                    if !permission_map.contains_key(name) {
                        invalid.insert(name.clone());
                    }
                }
            }
            if let Some(deny) = &site.deny {
                for name in deny {
                    if !permission_map.contains_key(name) {
                        invalid.insert(name.clone());
                    }
                }
            }
        }

        if invalid.is_empty() {
            Ok(())
        } else {
            Err(BrokerError::Internal(format!(
                "unknown permissions in policy: {}",
                invalid.into_iter().collect::<Vec<_>>().join(", ")
            )))
        }
    }

    async fn resolve_cached(&self, origin: &str) -> Result<CachedPolicy, BrokerError> {
        if let Some(entry) = self.cache.get(origin) {
            if !entry.value().is_expired() {
                return Ok(entry.value().clone());
            }
            self.cache.remove(origin);
        }

        let resolved = {
            let guard = self.store.read().await;
            guard.resolve(origin)
        }
        .ok_or_else(|| BrokerError::PolicyDenied(format!("no policy for {origin}")))?;

        let expires_at = resolved.ttl.map(|ttl| Instant::now() + ttl);
        let cached = CachedPolicy {
            template: resolved.template.clone(),
            ttl: resolved.ttl,
            expires_at,
        };
        self.cache.insert(origin.to_string(), cached.clone());
        Ok(cached)
    }

    async fn apply_transport(
        &self,
        origin: &str,
        decision: &AuthzDecision,
    ) -> Result<(), BrokerError> {
        if decision.allowed.is_empty() && decision.denied.is_empty() && decision.missing.is_empty()
        {
            return Ok(());
        }

        let transport = {
            let guard = self.transport.read().await;
            guard.clone()
        };

        let Some(client) = transport else {
            return Ok(());
        };

        let map_guard = self.permission_map.read().await;
        let permission_map = map_guard.as_ref();

        let grant = translate_names(permission_map, &decision.allowed)?;
        let mut revoke_source = decision.denied.clone();
        revoke_source.extend(decision.missing.clone());
        let revoke = translate_names(permission_map, &revoke_source)?;
        drop(map_guard);

        if grant.is_empty() && revoke.is_empty() {
            return Ok(());
        }

        client.apply_permissions(origin, &grant, &revoke).await?;
        Ok(())
    }

    fn publish_event(&self, origin: &str, decision: &AuthzDecision) {
        let event = AuditEvent {
            origin: origin.to_string(),
            decision: decision.kind,
            allowed: decision.allowed.clone(),
            denied: decision.denied.clone(),
            missing: decision.missing.clone(),
            ttl_ms: decision.ttl_ms,
            timestamp: SystemTime::now(),
        };

        if let Err(err) = self.events.send(event) {
            warn!(
                target = "permissions-broker",
                "failed to publish audit event: {err}"
            );
        }
    }
}

/// Trait describing the operations exposed to higher layers.
#[async_trait]
pub trait PermissionTransport: Send + Sync {
    async fn apply_permissions(
        &self,
        origin: &str,
        grant: &[String],
        revoke: &[String],
    ) -> Result<(), BrokerError>;
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
    fn subscribe(&self) -> broadcast::Receiver<AuditEvent>;
}

#[async_trait]
impl Broker for PermissionsBroker {
    async fn apply_policy(&self, origin: &str) -> Result<AuthzDecision, BrokerError> {
        let cached = self.resolve_cached(origin).await?;
        let decision = decision_from_template(&cached.template, None, cached.ttl);
        self.apply_transport(origin, &decision).await?;
        self.publish_event(origin, &decision);
        Ok(decision)
    }

    async fn ensure_for(
        &self,
        origin: &str,
        needs: &[String],
    ) -> Result<AuthzDecision, BrokerError> {
        let cached = self.resolve_cached(origin).await?;
        let decision = decision_from_template(&cached.template, Some(needs), cached.ttl);
        self.apply_transport(origin, &decision).await?;
        self.publish_event(origin, &decision);
        Ok(decision)
    }

    async fn revoke(&self, origin: &str, _which: Option<Vec<String>>) -> Result<(), BrokerError> {
        self.cache.remove(origin);
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<AuditEvent> {
        PermissionsBroker::subscribe(self)
    }
}

fn translate_names(
    map: Option<&PermissionMap>,
    names: &[String],
) -> Result<Vec<String>, BrokerError> {
    let mut seen = HashSet::new();
    let mut translated = Vec::new();
    for name in names {
        if !seen.insert(name.clone()) {
            continue;
        }
        let value = if let Some(map) = map {
            map.get(name).cloned().ok_or_else(|| {
                BrokerError::Internal(format!("permission '{name}' missing in permission map"))
            })?
        } else {
            name.clone()
        };
        translated.push(value);
    }
    Ok(translated)
}

fn decision_from_template(
    template: &PolicyTemplate,
    needs: Option<&[String]>,
    ttl: Option<Duration>,
) -> AuthzDecision {
    let mut allowed = template.allow.clone();
    let denied = template.deny.clone();
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
