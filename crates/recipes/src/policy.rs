use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecPolicyView {
    pub enabled: bool,
    pub caps: Caps,
    pub thresholds: Thresholds,
    pub freshness_tau_sec: u64,
    pub rollout: Rollout,
    pub hygiene: HygieneCfg,
    pub privacy: PrivacyCfg,
    pub embed: EmbedCfg,
    pub io: IoCfg,
}

impl Default for RecPolicyView {
    fn default() -> Self {
        Self {
            enabled: true,
            caps: Caps::default(),
            thresholds: Thresholds::default(),
            freshness_tau_sec: 86_400,
            rollout: Rollout::default(),
            hygiene: HygieneCfg::default(),
            privacy: PrivacyCfg::default(),
            embed: EmbedCfg::default(),
            io: IoCfg::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Caps {
    pub max_recipes_per_site: usize,
    pub max_vectors_per_site: usize,
    pub max_edges_per_site: usize,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            max_recipes_per_site: 2_000,
            max_vectors_per_site: 10_000,
            max_edges_per_site: 20_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Thresholds {
    pub suggest: f32,
    pub activate_quality: f32,
    pub activate_safety: f32,
    pub min_support_n: usize,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            suggest: 0.6,
            activate_quality: 0.8,
            activate_safety: 0.9,
            min_support_n: 3,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RollMode {
    Canary,
    Immediate,
    Manual,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rollout {
    pub default: RollMode,
}

impl Default for Rollout {
    fn default() -> Self {
        Self {
            default: RollMode::Canary,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HygieneCfg {
    pub dedup_vec_sim_th: f32,
    pub drift_dom_hash_th: f32,
    pub conflict_win_margin: f32,
    pub decay_step: f32,
    pub schedule_min_sec: u64,
}

impl Default for HygieneCfg {
    fn default() -> Self {
        Self {
            dedup_vec_sim_th: 0.92,
            drift_dom_hash_th: 0.15,
            conflict_win_margin: 0.1,
            decay_step: 0.05,
            schedule_min_sec: 300,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrivacyCfg {
    pub mask_regex: Vec<String>,
    pub forbid_paths: Vec<String>,
    pub high_risk_intents: Vec<String>,
}

impl Default for PrivacyCfg {
    fn default() -> Self {
        Self {
            mask_regex: vec![],
            forbid_paths: vec!["/auth".into(), "/pay".into()],
            high_risk_intents: vec!["payment".into(), "authentication".into()],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmbedCfg {
    pub dim: usize,
    pub provider: EmbedProvider,
    #[serde(default)]
    pub ann_engine: AnnBackend,
}

impl Default for EmbedCfg {
    fn default() -> Self {
        Self {
            dim: 256,
            provider: EmbedProvider::Rules,
            ann_engine: AnnBackend::InMemory,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EmbedProvider {
    Rules,
    External,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AnnBackend {
    InMemory,
    HnswStub,
}

impl Default for AnnBackend {
    fn default() -> Self {
        AnnBackend::InMemory
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IoCfg {
    pub root: PathBuf,
    pub snapshot_keep_versions: usize,
    pub snapshot_path: PathBuf,
    pub auto_persist: bool,
}

impl Default for IoCfg {
    fn default() -> Self {
        Self {
            root: PathBuf::from("./recipes"),
            snapshot_keep_versions: 5,
            snapshot_path: PathBuf::from("./recipes/recipes_snapshot.json"),
            auto_persist: true,
        }
    }
}
