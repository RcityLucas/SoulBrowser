use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::model::{MouseBtn, WaitTier};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClickPolicyView {
    pub enabled: bool,
    pub allow_self_heal: bool,
    pub allowed_buttons: Vec<MouseBtn>,
    pub max_offset_px: i32,
    pub wait_default: WaitTier,
    pub timeouts: ClickTimeouts,
}

impl Default for ClickPolicyView {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_self_heal: true,
            allowed_buttons: vec![MouseBtn::Left],
            max_offset_px: 32,
            wait_default: WaitTier::Auto,
            timeouts: ClickTimeouts::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClickTimeouts {
    pub precheck_ms: u64,
    pub after_click_ms: u64,
    pub domready_ms: u64,
}

impl ClickTimeouts {
    pub fn wait_for(&self, tier: WaitTier) -> Duration {
        match tier {
            WaitTier::Auto | WaitTier::DomReady => Duration::from_millis(self.domready_ms),
            WaitTier::None => Duration::from_millis(0),
        }
    }

    pub fn precheck(&self) -> Duration {
        Duration::from_millis(self.precheck_ms)
    }

    pub fn after_click(&self) -> Duration {
        Duration::from_millis(self.after_click_ms)
    }
}

impl Default for ClickTimeouts {
    fn default() -> Self {
        Self {
            precheck_ms: 2000,
            after_click_ms: 250,
            domready_ms: 3000,
        }
    }
}
