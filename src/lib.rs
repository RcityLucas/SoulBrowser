//! SoulBrowser library
//!
//! Exposes modules for integration testing

pub mod agent;
pub mod app_context;
pub mod auth;
pub mod browser_impl;
pub mod config;
pub mod errors;
pub mod intent;
pub mod interceptors;
pub mod judge;
pub mod l0_bridge;
pub mod llm;
pub mod metrics;
pub mod observation;
pub mod parsers;
pub mod plugin_registry;
pub mod policy;
pub mod replan;
pub mod self_heal;
pub mod storage;
pub mod structured_output;
pub mod task_status;
pub mod tools;
pub mod types;
pub mod watchdogs;

// Re-export commonly used types for external use
pub use browser_impl::{Browser, BrowserConfig, L0Protocol, L1BrowserManager, Page};
pub use types::BrowserType;
