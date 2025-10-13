//! Configuration management module
//!
//! Manages application configuration using soulbase-config
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
pub use soulbase_config::model::ConfigValue;
use soulbase_config::model::{ConfigMap, NamespaceId};
use std::path::PathBuf;

/// Browser configuration using soulbase-config
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserConfiguration {
    namespace: NamespaceId,
    values: ConfigMap,
}

impl BrowserConfiguration {
    /// Create new browser configuration
    pub fn new() -> Self {
        Self {
            namespace: NamespaceId("browser".to_string()),
            values: ConfigMap::new(),
        }
    }

    /// Set browser type
    pub fn set_browser_type(&mut self, browser_type: &str) {
        self.values.insert(
            "browser_type".to_string(),
            ConfigValue::String(browser_type.to_string()),
        );
    }

    /// Set headless mode
    pub fn set_headless(&mut self, headless: bool) {
        self.values
            .insert("headless".to_string(), ConfigValue::Bool(headless));
    }

    /// Set window size
    pub fn set_window_size(&mut self, width: u32, height: u32) {
        let size = serde_json::json!({
            "width": width,
            "height": height,
        });
        self.values.insert("window_size".to_string(), size);
    }

    /// Set devtools
    pub fn set_devtools(&mut self, devtools: bool) {
        self.values
            .insert("devtools".to_string(), ConfigValue::Bool(devtools));
    }

    /// Generic set method for any configuration value
    pub fn set(
        &mut self,
        key: String,
        value: ConfigValue,
    ) -> Result<(), crate::errors::SoulBrowserError> {
        self.values.insert(key, value);
        Ok(())
    }

    /// Load default configuration values
    pub fn load_defaults(&mut self) {
        self.values
            .insert("browser.headless".to_string(), ConfigValue::Bool(false));
        self.values
            .insert("browser.devtools".to_string(), ConfigValue::Bool(false));
        self.values.insert(
            "browser.type".to_string(),
            ConfigValue::String("chromium".to_string()),
        );
        self.values.insert(
            "storage.type".to_string(),
            ConfigValue::String("memory".to_string()),
        );
        self.values.insert(
            "auth.mode".to_string(),
            ConfigValue::String("minimal".to_string()),
        );
    }

    /// Get browser type
    pub fn browser_type(&self) -> Option<String> {
        self.values
            .get("browser_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// Get headless mode
    pub fn headless(&self) -> bool {
        self.values
            .get("headless")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    /// Get window size
    pub fn window_size(&self) -> Option<(u32, u32)> {
        self.values.get("window_size").and_then(|v| {
            let width = v.get("width")?.as_u64()? as u32;
            let height = v.get("height")?.as_u64()? as u32;
            Some((width, height))
        })
    }

    /// Get devtools setting
    pub fn devtools(&self) -> bool {
        self.values
            .get("devtools")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Convert to ConfigMap for soulbase-config
    pub fn to_config_map(&self) -> ConfigMap {
        self.values.clone()
    }

    /// Load from ConfigMap
    pub fn from_config_map(map: ConfigMap) -> Self {
        Self {
            namespace: NamespaceId("browser".to_string()),
            values: map,
        }
    }
}

/// Migration helper to convert old BrowserConfig to new system
pub fn migrate_browser_config(
    browser_type: crate::BrowserType,
    headless: bool,
    window_size: Option<(u32, u32)>,
    devtools: bool,
) -> BrowserConfiguration {
    let mut config = BrowserConfiguration::new();

    // Convert BrowserType enum to string
    let browser_str = match browser_type {
        crate::BrowserType::Chromium => "chromium",
        crate::BrowserType::Chrome => "chrome",
        crate::BrowserType::Firefox => "firefox",
        crate::BrowserType::Safari => "safari",
        crate::BrowserType::Edge => "edge",
    };

    config.set_browser_type(browser_str);
    config.set_headless(headless);

    if let Some((width, height)) = window_size {
        config.set_window_size(width, height);
    }

    config.set_devtools(devtools);

    config
}

/// Load configuration from multiple sources using soulbase-config
pub async fn load_configuration(
    config_file: Option<PathBuf>,
) -> Result<BrowserConfiguration, Box<dyn std::error::Error>> {
    let mut values = ConfigMap::new();

    // Default configuration
    values.insert(
        "browser_type".to_string(),
        ConfigValue::String("chrome".to_string()),
    );
    values.insert("headless".to_string(), ConfigValue::Bool(false));
    values.insert("devtools".to_string(), ConfigValue::Bool(false));

    // Load from file if provided
    if let Some(path) = config_file {
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let file_values: ConfigMap = serde_json::from_str(&content)?;

            // Merge file values
            for (key, value) in file_values {
                values.insert(key, value);
            }
        }
    }

    // Environment variable overrides
    if let Ok(browser) = std::env::var("SOUL_BROWSER_TYPE") {
        values.insert("browser_type".to_string(), ConfigValue::String(browser));
    }

    if let Ok(headless) = std::env::var("SOUL_HEADLESS") {
        if let Ok(b) = headless.parse::<bool>() {
            values.insert("headless".to_string(), ConfigValue::Bool(b));
        }
    }

    Ok(BrowserConfiguration::from_config_map(values))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browser_configuration() {
        let mut config = BrowserConfiguration::new();
        config.set_browser_type("firefox");
        config.set_headless(true);
        config.set_window_size(1920, 1080);
        config.set_devtools(false);

        assert_eq!(config.browser_type(), Some("firefox".to_string()));
        assert_eq!(config.headless(), true);
        assert_eq!(config.window_size(), Some((1920, 1080)));
        assert_eq!(config.devtools(), false);
    }

    #[test]
    fn test_migration() {
        let config =
            migrate_browser_config(crate::BrowserType::Chrome, true, Some((1024, 768)), true);

        assert_eq!(config.browser_type(), Some("chrome".to_string()));
        assert_eq!(config.headless(), true);
        assert_eq!(config.window_size(), Some((1024, 768)));
        assert_eq!(config.devtools(), true);
    }
}
