//! Policy and needs definitions for the permissions broker.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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
