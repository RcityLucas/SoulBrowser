//! Extensions bridge policy configuration.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BridgePolicyFile {
    pub version: u32,
    pub extensions: Vec<BridgePolicyEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BridgePolicyEntry {
    pub id: String,
    pub allow_ops: Vec<String>,
    pub deny_ops: Vec<String>,
    pub scopes: Vec<String>,
    pub ttl: Option<String>,
}
