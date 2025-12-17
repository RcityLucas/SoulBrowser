#![allow(clippy::module_inception)]

#[cfg(not(feature = "soulbase"))]
mod soulbase_shim;
#[cfg(not(feature = "soulbase"))]
pub use soulbase_shim::{
    soulbase_auth, soulbase_config, soulbase_errors, soulbase_interceptors, soulbase_storage,
    soulbase_tools, soulbase_types,
};

pub mod agent;
pub mod analytics;
pub mod app_context;
pub mod app_settings;
pub mod auth;
pub mod automation;
pub mod browser_impl;
pub mod chat_support;
pub mod config;
pub mod console_fixture;
pub mod errors;
pub mod export;
pub mod gateway;
pub mod gateway_policy;
pub mod integration;
pub mod intent;
pub mod interceptors;
pub mod judge;
pub mod kernel;
pub mod l0_bridge;
pub mod llm;
pub mod metrics;
pub mod observation;
pub mod parsers;
pub mod perception_service;
pub mod plugin_registry;
pub mod policy;
pub mod replan;
pub mod replay;
pub mod runtime;
pub mod self_heal;
pub mod server;
pub mod storage;
pub mod structured_output;
pub mod task_status;
pub mod task_store;
pub mod tools;
pub mod types;
pub mod utils;
pub mod visualization;
pub mod watchdogs;

pub use app_settings::{
    Config, PerformanceConfig, PerformanceThresholds, RecordingConfigOptions, SoulConfig,
};
pub use browser_impl::{Browser, BrowserConfig, L0Protocol, L1BrowserManager, Page};
pub use chat_support::plan_payload;
pub use gateway::GatewayOptions;
pub use kernel::{Kernel, ServeOptions};
pub use server::ServeSurfacePreset;
pub use types::BrowserType;
pub use utils::{
    build_exec_route, collect_events, ensure_real_chrome_enabled, wait_for_page_ready,
};

pub const CONSOLE_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../static/console.html"
));
