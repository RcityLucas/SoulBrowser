use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::model::WaitTier;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TypePolicyView {
    pub enabled: bool,
    pub allow_self_heal: bool,
    pub allow_paste: bool,
    pub max_text_len: usize,
    pub wait_default: WaitTier,
    pub timeouts: TypeTimeouts,
}

impl Default for TypePolicyView {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_self_heal: true,
            allow_paste: false,
            max_text_len: 4000,
            wait_default: WaitTier::Auto,
            timeouts: TypeTimeouts::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TypeTimeouts {
    pub precheck_ms: u64,
    pub typing_ms: u64,
    pub domready_ms: u64,
}

impl TypeTimeouts {
    pub fn precheck(&self) -> Duration {
        Duration::from_millis(self.precheck_ms)
    }

    pub fn typing(&self) -> Duration {
        Duration::from_millis(self.typing_ms)
    }

    pub fn wait_for(&self, tier: WaitTier) -> Duration {
        match tier {
            WaitTier::Auto | WaitTier::DomReady => Duration::from_millis(self.domready_ms),
            WaitTier::None => Duration::from_millis(0),
        }
    }
}

impl Default for TypeTimeouts {
    fn default() -> Self {
        Self {
            precheck_ms: 2000,
            typing_ms: 5000,
            domready_ms: 3000,
        }
    }
}
