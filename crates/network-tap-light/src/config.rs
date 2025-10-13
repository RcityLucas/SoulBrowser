//! Configuration types for the network tap (light).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TapConfig {
    pub window_ms: u64,
    pub quiet_window_ms: u64,
    pub min_publish_interval_ms: u64,
}

impl Default for TapConfig {
    fn default() -> Self {
        Self {
            window_ms: 250,
            quiet_window_ms: 1000,
            min_publish_interval_ms: 500,
        }
    }
}
