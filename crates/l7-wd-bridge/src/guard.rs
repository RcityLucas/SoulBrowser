use axum::http::HeaderMap;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

use crate::errors::{BridgeError, BridgeResult};
use crate::policy::{TenantPolicy, WebDriverBridgePolicyHandle};

#[allow(dead_code)]
#[derive(Clone)]
pub struct Guard {
    _policy: WebDriverBridgePolicyHandle,
    concurrency: Arc<DashMap<String, Arc<Semaphore>>>,
}

impl Guard {
    pub fn new(policy: WebDriverBridgePolicyHandle) -> Self {
        Self {
            _policy: policy,
            concurrency: Arc::new(DashMap::new()),
        }
    }

    pub fn check_headers(&self, _headers: &HeaderMap) -> BridgeResult<()> {
        Ok(())
    }

    pub fn acquire(&self, tenant: &TenantPolicy) -> BridgeResult<OwnedSemaphorePermit> {
        let semaphore = self.get_or_create(tenant);
        match semaphore.clone().try_acquire_owned() {
            Ok(permit) => Ok(permit),
            Err(TryAcquireError::NoPermits) => Err(BridgeError::Forbidden),
            Err(TryAcquireError::Closed) => Err(BridgeError::Internal),
        }
    }

    fn get_or_create(&self, tenant: &TenantPolicy) -> Arc<Semaphore> {
        use dashmap::mapref::entry::Entry;
        match self.concurrency.entry(tenant.id.clone()) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                let semaphore = Arc::new(Semaphore::new(tenant.concurrency_max as usize));
                entry.insert(semaphore.clone());
                semaphore
            }
        }
    }
}
