use crate::manifest::PluginManifest;
use crate::policy::{PluginPolicyHandle, PluginPolicyView};

#[derive(Clone)]
pub struct KillSwitch {
    policy: PluginPolicyHandle,
}

impl KillSwitch {
    pub fn new(policy: PluginPolicyHandle) -> Self {
        Self { policy }
    }

    pub fn is_blocked(&self, manifest: &PluginManifest) -> bool {
        let view: PluginPolicyView = self.policy.snapshot();
        view.kill_switch
            .iter()
            .any(|prefix| manifest.name.starts_with(prefix))
    }
}
