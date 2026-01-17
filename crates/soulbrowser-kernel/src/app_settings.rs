use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::server::ServeSurfacePreset;
use crate::types::BrowserType;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub default_browser: BrowserType,
    pub default_headless: bool,
    pub output_dir: PathBuf,
    pub session_timeout: u64,
    pub soul: SoulConfig,
    pub recording: RecordingConfigOptions,
    pub performance: PerformanceConfig,
    #[serde(default)]
    pub policy_paths: Vec<PathBuf>,
    #[serde(default)]
    pub strict_authorization: bool,
    #[serde(default)]
    pub serve_surface: Option<ServeSurfacePreset>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SoulConfig {
    pub enabled: bool,
    pub model: String,
    /// Legacy single API key field (for backwards compatibility)
    pub api_key: Option<String>,
    pub prompts_dir: Option<PathBuf>,
    /// Provider-specific API keys
    #[serde(default)]
    pub providers: ProvidersConfig,
}

/// Configuration for multiple LLM providers
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ProvidersConfig {
    pub openai: Option<ProviderConfig>,
    pub zhipu: Option<ProviderConfig>,
    pub anthropic: Option<ProviderConfig>,
    pub deepseek: Option<ProviderConfig>,
    pub gemini: Option<ProviderConfig>,
}

/// Configuration for a single LLM provider
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub api_base: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RecordingConfigOptions {
    pub screenshots: bool,
    pub video: bool,
    pub network: bool,
    pub video_quality: String,
    pub screenshot_format: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PerformanceConfig {
    pub enabled: bool,
    pub sampling_rate: f64,
    pub thresholds: PerformanceThresholds,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PerformanceThresholds {
    pub page_load_time: u64,
    pub first_contentful_paint: u64,
    pub largest_contentful_paint: u64,
    pub cumulative_layout_shift: f64,
    pub first_input_delay: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_browser: BrowserType::Chromium,
            default_headless: false,
            output_dir: PathBuf::from("./soulbrowser-output"),
            session_timeout: 300,
            soul: SoulConfig {
                enabled: false,
                model: "gpt-4".to_string(),
                api_key: None,
                prompts_dir: None,
                providers: ProvidersConfig::default(),
            },
            recording: RecordingConfigOptions {
                screenshots: true,
                video: false,
                network: true,
                video_quality: "high".to_string(),
                screenshot_format: "png".to_string(),
            },
            performance: PerformanceConfig {
                enabled: true,
                sampling_rate: 1.0,
                thresholds: PerformanceThresholds {
                    page_load_time: 3000,
                    first_contentful_paint: 1500,
                    largest_contentful_paint: 2500,
                    cumulative_layout_shift: 0.1,
                    first_input_delay: 100,
                },
            },
            policy_paths: Vec::new(),
            strict_authorization: false,
            serve_surface: None,
        }
    }
}
