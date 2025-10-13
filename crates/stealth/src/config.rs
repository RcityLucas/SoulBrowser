//! Configuration and policy definitions for stealth profiles and tempo plans.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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
    pub mouse: HashMap<String, f64>,
    pub typing: HashMap<String, f64>,
    pub scroll: HashMap<String, f64>,
}
