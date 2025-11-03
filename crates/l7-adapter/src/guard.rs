use crate::errors::AdapterError;
use crate::policy::TenantPolicy;
use dashmap::DashMap;
use governor::{
    clock::DefaultClock, state::direct::NotKeyed, state::InMemoryState, Quota, RateLimiter,
};
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

const UNLIMITED_CONCURRENCY: usize = usize::MAX >> 1;

type ArcLimiter = Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>;

pub struct RequestGuard {
    guards: DashMap<String, Arc<TenantGuard>>,
}

impl RequestGuard {
    pub fn new() -> Self {
        Self {
            guards: DashMap::new(),
        }
    }

    pub fn enter(&self, tenant: &TenantPolicy) -> Result<RequestPermit, AdapterError> {
        let guard = self.guard_for(tenant);
        guard.check_rate()?;
        let permit = guard.try_acquire()?;
        Ok(RequestPermit {
            _guard: guard,
            _permit: permit,
        })
    }

    fn guard_for(&self, tenant: &TenantPolicy) -> Arc<TenantGuard> {
        use dashmap::mapref::entry::Entry;
        match self.guards.entry(tenant.id.clone()) {
            Entry::Occupied(mut occ) => {
                if occ.get().needs_update(tenant) {
                    let new_guard = Arc::new(TenantGuard::new(tenant));
                    occ.insert(new_guard.clone());
                    new_guard
                } else {
                    occ.get().clone()
                }
            }
            Entry::Vacant(vac) => {
                let guard = Arc::new(TenantGuard::new(tenant));
                vac.insert(guard.clone());
                guard
            }
        }
    }
}

pub struct RequestPermit {
    _guard: Arc<TenantGuard>,
    _permit: OwnedSemaphorePermit,
}

struct TenantGuard {
    limiter: Option<ArcLimiter>,
    semaphore: Arc<Semaphore>,
    concurrency_max: u32,
    rate_limit_rps: u32,
}

impl TenantGuard {
    fn new(tenant: &TenantPolicy) -> Self {
        let (concurrency_max, rate_limit_rps) = (tenant.concurrency_max, tenant.rate_limit_rps);
        let permits = if concurrency_max == 0 {
            UNLIMITED_CONCURRENCY
        } else {
            concurrency_max as usize
        };
        let semaphore = Arc::new(Semaphore::new(permits));
        let limiter = NonZeroU32::new(rate_limit_rps)
            .map(|quota| Arc::new(RateLimiter::direct(Quota::per_second(quota))));
        Self {
            limiter,
            semaphore,
            concurrency_max,
            rate_limit_rps,
        }
    }

    fn needs_update(&self, tenant: &TenantPolicy) -> bool {
        self.concurrency_max != tenant.concurrency_max
            || self.rate_limit_rps != tenant.rate_limit_rps
    }

    fn check_rate(&self) -> Result<(), AdapterError> {
        if let Some(limiter) = &self.limiter {
            limiter.check().map_err(|_| AdapterError::TooManyRequests)?;
        }
        Ok(())
    }

    fn try_acquire(&self) -> Result<OwnedSemaphorePermit, AdapterError> {
        match self.semaphore.clone().try_acquire_owned() {
            Ok(permit) => Ok(permit),
            Err(TryAcquireError::NoPermits) => Err(AdapterError::ConcurrencyLimit),
            Err(TryAcquireError::Closed) => Err(AdapterError::Internal),
        }
    }
}
