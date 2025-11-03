//! Configuration and policy definitions for stealth profiles and tempo plans.

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to deserialize profile bundle: {0}")]
    Deserialize(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StealthProfileBundle {
    pub profiles: Vec<StealthProfile>,
    #[serde(default)]
    pub tempos: Vec<TempoPlan>,
    pub policy: Option<StealthPolicyFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StealthProfile {
    pub name: String,
    pub user_agent: String,
    #[serde(default)]
    pub accept_language: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub viewport: Option<Viewport>,
    #[serde(default)]
    pub touch: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: f64,
    #[serde(default)]
    pub mobile: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StealthPolicyFile {
    pub version: u32,
    pub defaults: StealthSitePolicy,
    pub sites: Vec<StealthSitePolicyEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StealthSitePolicy {
    pub profile: String,
    pub tempo: String,
    pub ttl: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StealthSitePolicyEntry {
    pub match_pattern: String,
    pub profile: Option<String>,
    pub tempo: Option<String>,
    pub ttl: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TempoPlan {
    pub name: String,
    #[serde(default)]
    pub mouse: MouseTempoPlan,
    #[serde(default)]
    pub typing: TypingTempoPlan,
    #[serde(default)]
    pub scroll: ScrollTempoPlan,
    #[serde(default)]
    pub seed: Option<u64>,
}

impl Default for TempoPlan {
    fn default() -> Self {
        Self {
            name: "default".into(),
            mouse: MouseTempoPlan::default(),
            typing: TypingTempoPlan::default(),
            scroll: ScrollTempoPlan::default(),
            seed: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MouseTempoPlan {
    #[serde(default = "MouseTempoPlan::default_pre_delay_ms")]
    pub pre_delay_ms: u64,
    #[serde(default = "MouseTempoPlan::default_hover_ms")]
    pub hover_ms: u64,
    #[serde(default = "MouseTempoPlan::default_press_ms")]
    pub press_ms: u64,
    #[serde(default)]
    pub jitter_px: f64,
    #[serde(default = "MouseTempoPlan::default_path_points")]
    pub path_points: u8,
}

impl MouseTempoPlan {
    fn default_pre_delay_ms() -> u64 {
        120
    }

    fn default_hover_ms() -> u64 {
        90
    }

    fn default_press_ms() -> u64 {
        40
    }

    fn default_path_points() -> u8 {
        4
    }
}

impl Default for MouseTempoPlan {
    fn default() -> Self {
        Self {
            pre_delay_ms: Self::default_pre_delay_ms(),
            hover_ms: Self::default_hover_ms(),
            press_ms: Self::default_press_ms(),
            jitter_px: 3.5,
            path_points: Self::default_path_points(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TypingTempoPlan {
    #[serde(default = "TypingTempoPlan::default_per_char_ms")]
    pub per_char_ms: u64,
    #[serde(default = "TypingTempoPlan::default_jitter_ms")]
    pub jitter_ms: u64,
}

impl TypingTempoPlan {
    fn default_per_char_ms() -> u64 {
        140
    }

    fn default_jitter_ms() -> u64 {
        60
    }
}

impl Default for TypingTempoPlan {
    fn default() -> Self {
        Self {
            per_char_ms: Self::default_per_char_ms(),
            jitter_ms: Self::default_jitter_ms(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScrollTempoPlan {
    #[serde(default = "ScrollTempoPlan::default_step_px")]
    pub step_px: u32,
    #[serde(default = "ScrollTempoPlan::default_dwell_ms")]
    pub dwell_ms: u64,
    #[serde(default = "ScrollTempoPlan::default_jitter_ms")]
    pub jitter_ms: u64,
}

impl ScrollTempoPlan {
    fn default_step_px() -> u32 {
        320
    }

    fn default_dwell_ms() -> u64 {
        180
    }

    fn default_jitter_ms() -> u64 {
        80
    }
}

impl Default for ScrollTempoPlan {
    fn default() -> Self {
        Self {
            step_px: Self::default_step_px(),
            dwell_ms: Self::default_dwell_ms(),
            jitter_ms: Self::default_jitter_ms(),
        }
    }
}

pub fn load_bundle_from_reader<R: Read>(
    mut reader: R,
) -> Result<StealthProfileBundle, ConfigError> {
    let mut buf = String::new();
    reader.read_to_string(&mut buf)?;
    parse_bundle_str(&buf)
}

pub fn load_bundle_from_path(path: impl AsRef<Path>) -> Result<StealthProfileBundle, ConfigError> {
    let file = File::open(path.as_ref())?;
    load_bundle_from_reader(file)
}

pub fn parse_bundle_str(raw: &str) -> Result<StealthProfileBundle, ConfigError> {
    match serde_json::from_str(raw) {
        Ok(bundle) => Ok(bundle),
        Err(json_err) => serde_yaml::from_str(raw).map_err(|yaml_err| {
            ConfigError::Deserialize(format!(
                "json error: {}; yaml error: {}",
                json_err, yaml_err
            ))
        }),
    }
}
