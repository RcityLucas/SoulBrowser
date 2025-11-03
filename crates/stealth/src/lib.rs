//! SoulBrowser L0 stealth fingerprint & captcha channel scaffold.
//!
//! The eventual implementation will coordinate fingerprint profiles, tempo guidance, and captcha
//! decision routing. This placeholder defines high-level data contracts and traits so that other
//! layers can begin wiring while the concrete logic is developed.

pub mod config;

use crate::config::{StealthPolicyFile, StealthProfile, StealthProfileBundle, TempoPlan};
use async_trait::async_trait;
use cdp_adapter::{ids::PageId as AdapterPageId, Cdp};
use dashmap::DashMap;
use parking_lot::RwLock;
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use thiserror::Error;
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

impl Default for TempoAdvice {
    fn default() -> Self {
        Self {
            delay_ms: 120,
            path: None,
            step_px: None,
        }
    }
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
    async fn configure_page(&self, page: AdapterPageId, origin: &str) -> Result<(), StealthError>;
    fn tempo_advice(&self, op: &str) -> TempoAdvice;
    async fn detect_captcha(&self, origin: &str) -> Result<Vec<CaptchaChallenge>, StealthError>;
    async fn decide_captcha(
        &self,
        challenge: &CaptchaChallenge,
    ) -> Result<CaptchaDecision, StealthError>;
}

/// Placeholder runtime with in-memory catalog & applied profiles.
pub struct StealthRuntime {
    applied: DashMap<String, AppliedProfile>,
    catalog: Arc<RwLock<ProfileCatalog>>,
    profiles: Arc<RwLock<HashMap<String, StealthProfile>>>,
    tempos: Arc<RwLock<HashMap<String, TempoPlan>>>,
    policy: Arc<RwLock<Option<StealthPolicyFile>>>,
    adapter: Option<Arc<dyn Cdp + Send + Sync>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppliedProfile {
    pub profile_id: ProfileId,
    pub profile_name: String,
    pub tempo: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProfileCatalog {
    pub profiles: Vec<String>,
}

#[derive(Clone, Debug)]
struct ResolvedPolicyChoice {
    profile: String,
    tempo: String,
}

impl StealthRuntime {
    pub fn new() -> Self {
        Self::with_optional_adapter(None)
    }

    pub fn with_adapter(adapter: Arc<dyn Cdp + Send + Sync>) -> Self {
        Self::with_optional_adapter(Some(adapter))
    }

    fn with_optional_adapter(adapter: Option<Arc<dyn Cdp + Send + Sync>>) -> Self {
        Self {
            applied: DashMap::new(),
            catalog: Arc::new(RwLock::new(ProfileCatalog::default())),
            profiles: Arc::new(RwLock::new(HashMap::new())),
            tempos: Arc::new(RwLock::new(HashMap::new())),
            policy: Arc::new(RwLock::new(None)),
            adapter,
        }
    }

    pub async fn load_catalog(&self, catalog: ProfileCatalog) {
        {
            let mut guard = self.catalog.write();
            *guard = catalog.clone();
        }

        let mut profiles = self.profiles.write();
        for name in &catalog.profiles {
            profiles
                .entry(name.clone())
                .or_insert_with(|| StealthProfile {
                    name: name.clone(),
                    user_agent: "".into(),
                    accept_language: None,
                    platform: None,
                    locale: None,
                    timezone: None,
                    viewport: None,
                    touch: false,
                });
        }
    }

    pub async fn load_bundle(&self, bundle: StealthProfileBundle) {
        let StealthProfileBundle {
            profiles: profile_defs,
            tempos: tempo_defs,
            policy,
        } = bundle;

        {
            let mut profiles = self.profiles.write();
            profiles.clear();
            for profile in profile_defs {
                profiles.insert(profile.name.clone(), profile);
            }
        }
        {
            let mut tempos = self.tempos.write();
            tempos.clear();
            for tempo in tempo_defs {
                tempos.insert(tempo.name.clone(), tempo);
            }
        }
        let profile_names = {
            let profiles = self.profiles.read();
            profiles.keys().cloned().collect::<Vec<_>>()
        };
        {
            let mut catalog = self.catalog.write();
            if catalog.profiles.is_empty() {
                catalog.profiles = profile_names;
            }
        }
        if let Some(policy) = policy {
            let mut guard = self.policy.write();
            *guard = Some(policy);
        }
    }

    async fn resolve_policy(&self, origin: &str) -> Option<ResolvedPolicyChoice> {
        let policy_guard = self.policy.read();
        let policy = policy_guard.as_ref()?;
        let mut choice = ResolvedPolicyChoice {
            profile: policy.defaults.profile.clone(),
            tempo: policy.defaults.tempo.clone(),
        };
        let mut best_len = 0usize;
        for entry in &policy.sites {
            if pattern_matches(&entry.match_pattern, origin) {
                let len = entry.match_pattern.len();
                if len >= best_len {
                    if let Some(profile) = &entry.profile {
                        choice.profile = profile.clone();
                    }
                    if let Some(tempo) = &entry.tempo {
                        choice.tempo = tempo.clone();
                    }
                    best_len = len;
                }
            }
        }
        Some(choice)
    }

    async fn choose_profile(&self, origin: &str) -> AppliedProfile {
        let policy_choice = self.resolve_policy(origin).await;
        let mut profile_name = policy_choice
            .as_ref()
            .map(|c| c.profile.clone())
            .unwrap_or_else(|| "default".into());

        {
            let catalog = self.catalog.read();
            if profile_name == "default" && !catalog.profiles.is_empty() {
                profile_name = catalog
                    .profiles
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "default".into());
            }
        }

        {
            let profiles = self.profiles.read();
            if !profiles.contains_key(&profile_name) {
                if let Some(first) = profiles.keys().next() {
                    profile_name = first.clone();
                }
            }
        }

        let tempo = policy_choice
            .as_ref()
            .map(|c| c.tempo.clone())
            .unwrap_or_else(|| {
                let tempos = self.tempos.read();
                tempos
                    .keys()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| "human_soft".into())
            });

        AppliedProfile {
            profile_id: ProfileId::new(),
            profile_name,
            tempo,
        }
    }

    pub fn applied_profile_for(&self, origin: &str) -> Option<AppliedProfile> {
        self.applied.get(origin).map(|entry| entry.value().clone())
    }

    async fn inject_profile(
        &self,
        adapter: &Arc<dyn Cdp + Send + Sync>,
        page: AdapterPageId,
        profile: &StealthProfile,
    ) -> Result<(), StealthError> {
        if !profile.user_agent.is_empty() {
            adapter
                .set_user_agent(
                    page,
                    &profile.user_agent,
                    profile.accept_language.as_deref(),
                    profile.platform.as_deref(),
                    profile.locale.as_deref(),
                )
                .await
                .map_err(map_adapter_error)?;
        }

        if let Some(timezone) = &profile.timezone {
            adapter
                .set_timezone(page, timezone)
                .await
                .map_err(map_adapter_error)?;
        }

        if let Some(viewport) = &profile.viewport {
            adapter
                .set_device_metrics(
                    page,
                    viewport.width,
                    viewport.height,
                    viewport.device_scale_factor,
                    viewport.mobile,
                )
                .await
                .map_err(map_adapter_error)?;
        }

        if profile.touch {
            adapter
                .set_touch_emulation(page, true)
                .await
                .map_err(map_adapter_error)?;
        }

        Ok(())
    }

    fn select_tempo_plan(&self) -> TempoPlan {
        let policy_tempo = {
            let policy = self.policy.read();
            policy.as_ref().map(|p| p.defaults.tempo.clone())
        };

        let applied_tempo = self
            .applied
            .iter()
            .next()
            .map(|entry| entry.value().tempo.clone());

        let desired_name = policy_tempo.or(applied_tempo);

        let tempos = self.tempos.read();
        if let Some(name) = desired_name {
            if let Some(plan) = tempos.get(&name) {
                return plan.clone();
            }
        }

        tempos
            .values()
            .next()
            .cloned()
            .unwrap_or_else(TempoPlan::default)
    }

    fn advice_from_plan(plan: &TempoPlan, op: &str) -> TempoAdvice {
        let op_trimmed = op.trim();
        let normalized = if op_trimmed.is_empty() {
            "click".to_string()
        } else {
            op_trimmed.to_ascii_lowercase()
        };

        let seed = Self::tempo_seed(plan, &normalized);
        let mut rng = StdRng::seed_from_u64(seed);

        match normalized.as_str() {
            "click" | "mouse.click" | "mouse.select" | "select" => {
                Self::mouse_advice(plan, &mut rng)
            }
            "type" | "typing" | "keyboard.type" => Self::typing_advice(plan, &mut rng),
            op if op.starts_with("scroll") => Self::scroll_advice(plan, &mut rng),
            _ => TempoAdvice::default(),
        }
    }

    fn tempo_seed(plan: &TempoPlan, op: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        plan.name.hash(&mut hasher);
        op.hash(&mut hasher);
        if let Some(seed) = plan.seed {
            seed.hash(&mut hasher);
        }
        hasher.finish()
    }

    fn mouse_advice(plan: &TempoPlan, rng: &mut StdRng) -> TempoAdvice {
        let hover_jitter = if plan.mouse.hover_ms > 0 {
            rng.gen_range(0..=plan.mouse.hover_ms)
        } else {
            0
        };
        let press_jitter = if plan.mouse.press_ms > 0 {
            rng.gen_range(0..=plan.mouse.press_ms)
        } else {
            0
        };

        let mut advice = TempoAdvice {
            delay_ms: plan.mouse.pre_delay_ms + hover_jitter + press_jitter,
            path: None,
            step_px: None,
        };

        let steps = plan.mouse.path_points.max(2) as usize;
        if plan.mouse.jitter_px > 0.0 && steps > 1 {
            let mut path = Vec::with_capacity(steps);
            for _ in 0..steps {
                let dx = rng.gen_range(-plan.mouse.jitter_px..=plan.mouse.jitter_px);
                let dy = rng.gen_range(-plan.mouse.jitter_px..=plan.mouse.jitter_px);
                path.push((dx.round() as i32, dy.round() as i32));
            }
            advice.path = Some(path);
        }

        advice
    }

    fn typing_advice(plan: &TempoPlan, rng: &mut StdRng) -> TempoAdvice {
        let jitter = if plan.typing.jitter_ms > 0 {
            rng.gen_range(0..=plan.typing.jitter_ms)
        } else {
            0
        };

        TempoAdvice {
            delay_ms: plan.typing.per_char_ms + jitter,
            path: None,
            step_px: None,
        }
    }

    fn scroll_advice(plan: &TempoPlan, rng: &mut StdRng) -> TempoAdvice {
        let jitter = if plan.scroll.jitter_ms > 0 {
            rng.gen_range(0..=plan.scroll.jitter_ms)
        } else {
            0
        };

        TempoAdvice {
            delay_ms: plan.scroll.dwell_ms + jitter,
            path: None,
            step_px: Some(plan.scroll.step_px),
        }
    }
}

#[async_trait]
impl StealthControl for StealthRuntime {
    async fn apply_stealth(&self, origin: &str) -> Result<ProfileId, StealthError> {
        let applied = self.choose_profile(origin).await;
        let profile_id = applied.profile_id.clone();
        self.applied.insert(origin.to_string(), applied);
        Ok(profile_id)
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

    async fn configure_page(&self, page: AdapterPageId, origin: &str) -> Result<(), StealthError> {
        let adapter = self
            .adapter
            .as_ref()
            .ok_or_else(|| StealthError::Internal("stealth adapter not configured".into()))?
            .clone();

        let profile_name = self
            .applied
            .get(origin)
            .map(|entry| entry.profile_name.clone())
            .ok_or_else(|| {
                StealthError::PolicyDenied(format!("no profile applied for {origin}"))
            })?;

        let profile = {
            let profiles = self.profiles.read();
            profiles.get(&profile_name).cloned().ok_or_else(|| {
                StealthError::Internal(format!(
                    "profile '{profile_name}' not found for origin {origin}"
                ))
            })?
        };

        self.inject_profile(&adapter, page, &profile).await
    }

    fn tempo_advice(&self, op: &str) -> TempoAdvice {
        let plan = self.select_tempo_plan();
        Self::advice_from_plan(&plan, op)
    }

    async fn detect_captcha(&self, _origin: &str) -> Result<Vec<CaptchaChallenge>, StealthError> {
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

fn pattern_matches(pattern: &str, origin: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(idx) = pattern.find('*') {
        let prefix = &pattern[..idx];
        let suffix = &pattern[idx + 1..];
        return origin.starts_with(prefix) && origin.ends_with(suffix);
    }
    origin == pattern
}

fn map_adapter_error(err: cdp_adapter::AdapterError) -> StealthError {
    let mut hint = err.hint.clone().unwrap_or_default();
    if hint.is_empty() {
        hint = format!("cdp error {:?}", err.kind);
    }
    StealthError::CdpIo(hint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        MouseTempoPlan, ScrollTempoPlan, StealthPolicyFile, StealthProfile, StealthProfileBundle,
        StealthSitePolicy, StealthSitePolicyEntry, TempoPlan, TypingTempoPlan,
    };

    fn sample_profile(name: &str) -> StealthProfile {
        StealthProfile {
            name: name.into(),
            user_agent: String::new(),
            accept_language: None,
            platform: None,
            locale: None,
            timezone: None,
            viewport: None,
            touch: false,
        }
    }

    #[tokio::test]
    async fn tempo_advice_respects_plan_defaults() {
        let runtime = StealthRuntime::new();
        let tempo = TempoPlan {
            name: "human_soft".into(),
            mouse: MouseTempoPlan {
                pre_delay_ms: 200,
                hover_ms: 0,
                press_ms: 0,
                jitter_px: 0.0,
                path_points: 2,
            },
            typing: TypingTempoPlan {
                per_char_ms: 333,
                jitter_ms: 0,
            },
            scroll: ScrollTempoPlan {
                step_px: 600,
                dwell_ms: 500,
                jitter_ms: 0,
            },
            seed: Some(42),
        };

        let policy = StealthPolicyFile {
            version: 1,
            defaults: StealthSitePolicy {
                profile: "default".into(),
                tempo: "human_soft".into(),
                ttl: None,
            },
            sites: Vec::new(),
        };

        runtime
            .load_bundle(StealthProfileBundle {
                profiles: vec![sample_profile("default")],
                tempos: vec![tempo],
                policy: Some(policy),
            })
            .await;

        let advice_click = runtime.tempo_advice("click");
        assert_eq!(advice_click.delay_ms, 200);
        assert!(advice_click.step_px.is_none());
        assert!(advice_click.path.is_none());

        let advice_type = runtime.tempo_advice("type");
        assert_eq!(advice_type.delay_ms, 333);
        assert!(advice_type.step_px.is_none());

        let advice_scroll = runtime.tempo_advice("scroll.down");
        assert_eq!(advice_scroll.step_px, Some(600));
        assert_eq!(advice_scroll.delay_ms, 500);
    }

    #[tokio::test]
    async fn tempo_advice_is_deterministic_with_seed() {
        let runtime = StealthRuntime::new();
        let tempo = TempoPlan {
            name: "soft_jitter".into(),
            mouse: MouseTempoPlan {
                pre_delay_ms: 120,
                hover_ms: 30,
                press_ms: 20,
                jitter_px: 1.5,
                path_points: 3,
            },
            typing: TypingTempoPlan {
                per_char_ms: 180,
                jitter_ms: 40,
            },
            scroll: ScrollTempoPlan {
                step_px: 420,
                dwell_ms: 240,
                jitter_ms: 60,
            },
            seed: Some(7),
        };

        let policy = StealthPolicyFile {
            version: 1,
            defaults: StealthSitePolicy {
                profile: "default".into(),
                tempo: "soft_jitter".into(),
                ttl: None,
            },
            sites: vec![StealthSitePolicyEntry {
                match_pattern: "*".into(),
                profile: None,
                tempo: None,
                ttl: None,
            }],
        };

        runtime
            .load_bundle(StealthProfileBundle {
                profiles: vec![sample_profile("default")],
                tempos: vec![tempo],
                policy: Some(policy),
            })
            .await;

        let first = runtime.tempo_advice("click");
        let second = runtime.tempo_advice("click");
        assert_eq!(first.delay_ms, second.delay_ms);
        assert_eq!(first.path, second.path);

        let first_type = runtime.tempo_advice("type");
        let second_type = runtime.tempo_advice("type");
        assert_eq!(first_type.delay_ms, second_type.delay_ms);
    }
}
