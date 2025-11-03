use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::model::{SelectMode, WaitTier};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectPolicyView {
    pub enabled: bool,
    pub allow_self_heal: bool,
    pub allowed_modes: Vec<SelectMode>,
    pub wait_default: WaitTier,
    pub timeouts: SelectTimeouts,
}

impl Default for SelectPolicyView {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_self_heal: true,
            allowed_modes: vec![SelectMode::Single, SelectMode::Multiple, SelectMode::Toggle],
            wait_default: WaitTier::Auto,
            timeouts: SelectTimeouts::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectTimeouts {
    pub precheck_ms: u64,
    pub select_ms: u64,
    pub domready_ms: u64,
}

impl SelectTimeouts {
    pub fn precheck(&self) -> Duration {
        Duration::from_millis(self.precheck_ms)
    }

    pub fn selection(&self) -> Duration {
        Duration::from_millis(self.select_ms)
    }

    pub fn wait_for(&self, tier: WaitTier) -> Duration {
        match tier {
            WaitTier::Auto | WaitTier::DomReady => Duration::from_millis(self.domready_ms),
            WaitTier::None => Duration::from_millis(0),
        }
    }
}

impl Default for SelectTimeouts {
    fn default() -> Self {
        Self {
            precheck_ms: 2000,
            select_ms: 3000,
            domready_ms: 3000,
        }
    }
}
