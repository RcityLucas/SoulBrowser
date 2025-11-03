use crate::manifest::{ManifestError, PluginManifest};
use crate::policy::{PluginPolicyHandle, PluginPolicyView};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginStatus {
    Disabled,
    Enabled,
    Blocked,
}

#[derive(Debug, Clone)]
pub struct PluginRecord {
    pub manifest: PluginManifest,
    pub status: PluginStatus,
}

#[derive(Clone)]
pub struct PluginRegistry {
    inner: Arc<RwLock<HashMap<String, PluginRecord>>>,
    policy: PluginPolicyHandle,
}

impl PluginRegistry {
    pub fn new(policy: PluginPolicyHandle) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            policy,
        }
    }

    pub fn upsert(&self, manifest: PluginManifest) -> Result<(), ManifestError> {
        manifest.validate()?;
        let mut guard = self.inner.write();
        let status = if self.is_killed(&manifest) {
            PluginStatus::Blocked
        } else if self.policy.snapshot().enable {
            PluginStatus::Enabled
        } else {
            PluginStatus::Disabled
        };
        guard.insert(manifest.name.clone(), PluginRecord { manifest, status });
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<PluginRecord> {
        self.inner.read().get(name).cloned()
    }

    fn is_killed(&self, manifest: &PluginManifest) -> bool {
        let policy: PluginPolicyView = self.policy.snapshot();
        policy
            .kill_switch
            .iter()
            .any(|pattern| manifest.name.starts_with(pattern))
    }
}
