//! Policy and needs definitions for the permissions broker.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Static policy definition file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyFile {
    pub version: u32,
    pub defaults: PolicyTemplate,
    pub sites: Vec<SitePolicy>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyTemplate {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub ttl: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SitePolicy {
    pub match_pattern: String,
    pub allow: Option<Vec<String>>,
    pub deny: Option<Vec<String>>,
    pub ttl: Option<String>,
    pub notes: Option<String>,
}

/// Needs expressed by upper layers before invoking sensitive operations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Needs {
    pub permissions: Vec<String>,
}

/// Mapping between logical policy names and CDP permission strings.
pub type PermissionMap = HashMap<String, String>;

/// Errors surfaced while loading policy or permission map configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to deserialize policy: {0}")]
    Deserialize(String),
}

pub fn load_policy_from_reader<R: Read>(mut reader: R) -> Result<PolicyFile, ConfigError> {
    let mut buf = String::new();
    reader.read_to_string(&mut buf)?;
    parse_policy_str(&buf)
}

pub fn load_policy_from_path(path: impl AsRef<Path>) -> Result<PolicyFile, ConfigError> {
    let file = File::open(path.as_ref())?;
    load_policy_from_reader(file)
}

pub fn parse_policy_str(raw: &str) -> Result<PolicyFile, ConfigError> {
    match serde_json::from_str(raw) {
        Ok(policy) => Ok(policy),
        Err(json_err) => serde_yaml::from_str(raw).map_err(|yaml_err| {
            ConfigError::Deserialize(format!(
                "json error: {}; yaml error: {}",
                json_err, yaml_err
            ))
        }),
    }
}

pub fn load_permission_map_from_reader<R: Read>(
    mut reader: R,
) -> Result<PermissionMap, ConfigError> {
    let mut buf = String::new();
    reader.read_to_string(&mut buf)?;
    parse_permission_map_str(&buf)
}

pub fn load_permission_map_from_path(path: impl AsRef<Path>) -> Result<PermissionMap, ConfigError> {
    let file = File::open(path.as_ref())?;
    load_permission_map_from_reader(file)
}

pub fn parse_permission_map_str(raw: &str) -> Result<PermissionMap, ConfigError> {
    match serde_json::from_str(raw) {
        Ok(map) => Ok(map),
        Err(json_err) => serde_yaml::from_str(raw).map_err(|yaml_err| {
            ConfigError::Deserialize(format!(
                "json error: {}; yaml error: {}",
                json_err, yaml_err
            ))
        }),
    }
}

/// Default permission map used when configuration files are missing.
pub fn default_permission_map() -> PermissionMap {
    let mut map = PermissionMap::new();
    map.insert("clipboard_read".to_string(), "clipboardRead".to_string());
    map.insert("clipboard_write".to_string(), "clipboardWrite".to_string());
    map.insert("notifications".to_string(), "notifications".to_string());
    map.insert("geolocation".to_string(), "geolocation".to_string());
    map.insert("camera".to_string(), "camera".to_string());
    map.insert("microphone".to_string(), "microphone".to_string());
    map
}

/// Default policy file used when configuration files are missing.
pub fn default_policy_file() -> PolicyFile {
    PolicyFile {
        version: 1,
        defaults: PolicyTemplate {
            allow: vec!["clipboard_read".into()],
            deny: vec![],
            ttl: Some("session".into()),
        },
        sites: vec![SitePolicy {
            match_pattern: "https://*.example.com".into(),
            allow: Some(vec!["clipboard_read".into(), "clipboard_write".into()]),
            deny: None,
            ttl: Some("30m".into()),
            notes: Some("Default example policy".into()),
        }],
    }
}
