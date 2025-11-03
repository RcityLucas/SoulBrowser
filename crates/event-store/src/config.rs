use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level policy snapshot consumed by the event store runtime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EsPolicyView {
    pub hot: HotCfg,
    pub cold: ColdCfg,
    pub redact: RedactRules,
    pub drop: DropPolicy,
    pub idempotency: IdempotencyCfg,
    pub privacy: PrivacyCfg,
}

impl Default for EsPolicyView {
    fn default() -> Self {
        Self {
            hot: HotCfg::default(),
            cold: ColdCfg::default(),
            redact: RedactRules::default(),
            drop: DropPolicy::default(),
            idempotency: IdempotencyCfg::default(),
            privacy: PrivacyCfg::default(),
        }
    }
}

/// Capacity knobs for in-memory rings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HotCfg {
    pub n_global: usize,
    pub n_session: usize,
    pub n_page: usize,
    pub n_task: usize,
    pub max_payload_bytes: usize,
}

impl Default for HotCfg {
    fn default() -> Self {
        Self {
            n_global: 64_000,
            n_session: 8_000,
            n_page: 8_000,
            n_task: 2_000,
            max_payload_bytes: 16 * 1024,
        }
    }
}

/// File-level configuration for the optional cold debug log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColdCfg {
    pub enabled: bool,
    pub root: PathBuf,
    pub rotate_bytes: u64,
    pub rotate_interval_min: u32,
    pub compress: bool,
    pub retain_gb: u64,
    pub retain_days: u32,
}

impl Default for ColdCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            root: PathBuf::from("./event-store"),
            rotate_bytes: 256 * 1024 * 1024,
            rotate_interval_min: 30,
            compress: true,
            retain_gb: 2,
            retain_days: 3,
        }
    }
}

/// Redaction options applied before persistence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RedactRules {
    pub mask_url_query: bool,
    pub mask_patterns: Vec<String>,
    pub max_text_len: usize,
}

impl Default for RedactRules {
    fn default() -> Self {
        Self {
            mask_url_query: true,
            mask_patterns: vec![],
            max_text_len: 256,
        }
    }
}

/// Priority list for lossy drops under pressure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DropPolicy {
    pub low_priority_kinds: Vec<String>,
    pub protected_kinds: Vec<String>,
    pub hot_high_watermark: f32,
}

impl DropPolicy {
    pub fn should_protect(&self, kind: &str) -> bool {
        self.protected_kinds.iter().any(|k| k == kind)
    }
}

impl Default for DropPolicy {
    fn default() -> Self {
        Self {
            low_priority_kinds: vec![
                "NR_SNAPSHOT".into(),
                "VIS_LAYOUT".into(),
                "PERF_LIGHT".into(),
                "CONSOLE_DEBUG".into(),
            ],
            protected_kinds: vec![
                "OBSERVATION".into(),
                "ACT".into(),
                "GATE".into(),
                "HEAL".into(),
                "NR_PACK".into(),
            ],
            hot_high_watermark: 0.9,
        }
    }
}

/// Configuration for idempotency tracking caches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IdempotencyCfg {
    pub lru_capacity: usize,
    pub bloom_bits: usize,
}

impl Default for IdempotencyCfg {
    fn default() -> Self {
        Self {
            lru_capacity: 512_000,
            bloom_bits: 1 << 22,
        }
    }
}

/// Privacy-related controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrivacyCfg {
    pub forbid_headers: Vec<String>,
}

impl Default for PrivacyCfg {
    fn default() -> Self {
        Self {
            forbid_headers: vec!["authorization".into(), "cookie".into(), "set-cookie".into()],
        }
    }
}
