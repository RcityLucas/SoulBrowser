//! SoulBrowser L0 stealth fingerprint & captcha channel scaffold.
//!
//! The eventual implementation will coordinate fingerprint profiles, tempo guidance, and captcha
//! decision routing. This placeholder defines high-level data contracts and traits so that other
//! layers can begin wiring while the concrete logic is developed.

pub mod config;

use async_trait::async_trait;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Identifier for a stealth profile.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ProfileId(pub Uuid);

impl ProfileId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Tempo advice returned to higher layers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TempoAdvice {
    pub delay_ms: u64,
    pub path: Option<Vec<(i32, i32)>>,
    pub step_px: Option<u32>,
}

/// Captcha challenge descriptor.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptchaChallenge {
    pub id: String,
    pub origin: String,
    pub kind: CaptchaKind,
}

/// Captcha decision record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptchaDecision {
    pub strategy: DecisionStrategy,
    pub timeout_ms: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum CaptchaKind {
    Checkbox,
    Image,
    Invisible,
    Slider,
    Other,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum DecisionStrategy {
    Manual,
    External,
    Skip,
}

#[derive(Clone, Debug, Error)]
pub enum StealthError {
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    #[error("cdp I/O failure: {0}")]
    CdpIo(String),
    #[error("internal error: {0}")]
    Internal(String),
}

#[async_trait]
pub trait StealthControl {
    async fn apply_stealth(&self, origin: &str) -> Result<ProfileId, StealthError>;
    async fn ensure_consistency(&self, origin: &str) -> Result<(), StealthError>;
    fn tempo_advice(&self, op: &str) -> TempoAdvice;
    async fn detect_captcha(&self, origin: &str) -> Result<Vec<CaptchaChallenge>, StealthError>;
    async fn decide_captcha(
        &self,
        challenge: &CaptchaChallenge,
    ) -> Result<CaptchaDecision, StealthError>;
}

/// Placeholder runtime with in-memory catalog & applied profiles.
#[derive(Default)]
pub struct StealthRuntime {
    applied: DashMap<String, AppliedProfile>,
    catalog: Arc<RwLock<ProfileCatalog>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppliedProfile {
    pub profile_id: ProfileId,
    pub tempo: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProfileCatalog {
    pub profiles: Vec<String>,
}

impl StealthRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn load_catalog(&self, catalog: ProfileCatalog) {
        let mut guard = self.catalog.write().await;
        *guard = catalog;
    }

    fn choose_profile(&self, origin: &str) -> AppliedProfile {
        let _ = origin;
        AppliedProfile {
            profile_id: ProfileId::new(),
            tempo: "human_soft".into(),
        }
    }

    pub fn applied_profile_for(&self, origin: &str) -> Option<AppliedProfile> {
        self.applied.get(origin).map(|entry| entry.value().clone())
    }
}

#[async_trait]
impl StealthControl for StealthRuntime {
    async fn apply_stealth(&self, origin: &str) -> Result<ProfileId, StealthError> {
        let AppliedProfile { profile_id, tempo } = self.choose_profile(origin);
        let id_clone = profile_id.clone();
        self.applied
            .insert(origin.to_string(), AppliedProfile { profile_id, tempo });
        Ok(id_clone)
    }

    async fn ensure_consistency(&self, origin: &str) -> Result<(), StealthError> {
        if self.applied.get(origin).is_some() {
            Ok(())
        } else {
            Err(StealthError::PolicyDenied(format!(
                "no profile applied for {origin}"
            )))
        }
    }

    fn tempo_advice(&self, _op: &str) -> TempoAdvice {
        TempoAdvice {
            delay_ms: 120,
            path: None,
            step_px: Some(240),
        }
    }

    async fn detect_captcha(&self, origin: &str) -> Result<Vec<CaptchaChallenge>, StealthError> {
        let _ = origin;
        Ok(Vec::new())
    }

    async fn decide_captcha(
        &self,
        _challenge: &CaptchaChallenge,
    ) -> Result<CaptchaDecision, StealthError> {
        Ok(CaptchaDecision {
            strategy: DecisionStrategy::Manual,
            timeout_ms: 20_000,
        })
    }
}
