use crate::errors::{PluginError, PluginResult};
use crate::manifest::PluginManifest;
use crate::policy::{PluginPolicyHandle, PluginPolicyView};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

#[derive(Clone)]
pub struct PluginGuard {
    policy: PluginPolicyHandle,
    slots: Arc<DashMap<String, Arc<Semaphore>>>,
}

impl PluginGuard {
    pub fn new(policy: PluginPolicyHandle) -> Self {
        Self {
            policy,
            slots: Arc::new(DashMap::new()),
        }
    }

    pub fn check_install(&self, manifest: &PluginManifest) -> PluginResult<()> {
        let policy = self.policy.snapshot();
        if !policy.enable {
            return Err(PluginError::Disabled);
        }
        if policy
            .kill_switch
            .iter()
            .any(|pattern| manifest.name.starts_with(pattern))
        {
            return Err(PluginError::Blocked);
        }
        Ok(())
    }

    pub async fn acquire(&self, plugin: &str) -> PluginResult<OwnedSemaphorePermit> {
        let policy = self.policy.snapshot();
        let semaphore = self.get_or_create_semaphore(plugin, &policy);
        match semaphore.clone().try_acquire_owned() {
            Ok(permit) => Ok(permit),
            Err(TryAcquireError::NoPermits) => {
                Err(PluginError::Sandbox("concurrency limit".into()))
            }
            Err(TryAcquireError::Closed) => Err(PluginError::Sandbox("semaphore closed".into())),
        }
    }

    fn get_or_create_semaphore(&self, plugin: &str, policy: &PluginPolicyView) -> Arc<Semaphore> {
        use dashmap::mapref::entry::Entry;
        match self.slots.entry(plugin.to_string()) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                let semaphore = Arc::new(Semaphore::new(policy.concurrency as usize));
                entry.insert(semaphore.clone());
                semaphore
            }
        }
    }
}
