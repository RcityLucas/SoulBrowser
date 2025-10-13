//! SoulBrowser library
//!
//! Exposes modules for integration testing

pub mod app_context;
pub mod auth;
pub mod browser_impl;
pub mod config;
pub mod errors;
pub mod interceptors;
pub mod l0_bridge;
pub mod policy;
pub mod storage;
pub mod tools;
pub mod types;

// Re-export commonly used types for external use
pub use browser_impl::{Browser, BrowserConfig, L0Protocol, L1BrowserManager, Page};
pub use types::BrowserType;
