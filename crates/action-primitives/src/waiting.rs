//! Built-in waiting mechanisms for action primitives

use crate::{errors::ActionError, types::WaitTier};
use async_trait::async_trait;
use cdp_adapter::{commands::WaitGate, Cdp, CdpAdapter, PageId};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// Waiting strategy trait
#[async_trait]
pub trait WaitStrategy: Send + Sync {
    /// Execute the wait strategy
    async fn wait(
        &self,
        adapter: Arc<CdpAdapter>,
        page: PageId,
        tier: WaitTier,
    ) -> Result<(), ActionError>;
}

/// Default waiting strategy implementation
pub struct DefaultWaitStrategy {
    /// Timeout for DomReady tier (milliseconds)
    pub domready_timeout_ms: u64,

    /// Timeout for Idle tier (milliseconds)
    pub idle_timeout_ms: u64,

    /// Network idle quiet period (milliseconds)
    pub network_quiet_ms: u64,
}

impl Default for DefaultWaitStrategy {
    fn default() -> Self {
        Self {
            domready_timeout_ms: 5000, // 5 seconds for DOM ready
            idle_timeout_ms: 10000,    // 10 seconds for idle
            network_quiet_ms: 500,     // 500ms of network quiet
        }
    }
}

#[async_trait]
impl WaitStrategy for DefaultWaitStrategy {
    async fn wait(
        &self,
        adapter: Arc<CdpAdapter>,
        page: PageId,
        tier: WaitTier,
    ) -> Result<(), ActionError> {
        match tier {
            WaitTier::None => {
                debug!("WaitTier::None - no waiting");
                Ok(())
            }

            WaitTier::DomReady => {
                debug!("WaitTier::DomReady - waiting for DOM ready");
                self.wait_domready(adapter, page).await
            }

            WaitTier::Idle => {
                debug!("WaitTier::Idle - waiting for page idle");
                self.wait_idle(adapter, page).await
            }
        }
    }
}

impl DefaultWaitStrategy {
    /// Wait for DOM ready
    async fn wait_domready(
        &self,
        adapter: Arc<CdpAdapter>,
        page: PageId,
    ) -> Result<(), ActionError> {
        self.exec_wait_gate(
            adapter,
            page,
            WaitGate::DomReady,
            Duration::from_millis(self.domready_timeout_ms),
        )
        .await
    }

    /// Wait for page idle (DOM ready + network quiet)
    async fn wait_idle(&self, adapter: Arc<CdpAdapter>, page: PageId) -> Result<(), ActionError> {
        // First wait for DOM ready
        self.wait_domready(adapter.clone(), page).await?;

        // Then wait for network idle
        self.exec_wait_gate(
            adapter,
            page,
            WaitGate::NetworkQuiet {
                window_ms: self.network_quiet_ms,
                max_inflight: 0,
            },
            Duration::from_millis(self.idle_timeout_ms),
        )
        .await
    }

    async fn exec_wait_gate(
        &self,
        adapter: Arc<CdpAdapter>,
        page: PageId,
        gate: WaitGate,
        timeout: Duration,
    ) -> Result<(), ActionError> {
        let gate_json = serde_json::to_string(&gate).map_err(|err| {
            ActionError::Internal(format!("failed to serialize wait gate: {}", err))
        })?;

        adapter
            .wait_basic(page, gate_json, timeout)
            .await
            .map_err(|err| {
                warn!("wait gate {:?} failed: {}", gate, err);
                ActionError::CdpIo(err.to_string())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_wait_strategy_config() {
        let strategy = DefaultWaitStrategy::default();
        assert_eq!(strategy.domready_timeout_ms, 5000);
        assert_eq!(strategy.idle_timeout_ms, 10000);
        assert_eq!(strategy.network_quiet_ms, 500);
    }

    #[test]
    fn test_wait_tier_default() {
        assert_eq!(WaitTier::default(), WaitTier::DomReady);
    }
}
