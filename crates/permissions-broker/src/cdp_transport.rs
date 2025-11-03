use std::sync::Arc;

use async_trait::async_trait;
use cdp_adapter::Cdp;

use crate::{BrokerError, PermissionTransport};

/// CDP-backed transport that applies permission decisions via Chromium DevTools Protocol.
pub struct CdpPermissionTransport {
    adapter: Arc<dyn Cdp + Send + Sync>,
}

impl CdpPermissionTransport {
    pub fn new(adapter: Arc<dyn Cdp + Send + Sync>) -> Self {
        Self { adapter }
    }
}

fn map_adapter_error(err: cdp_adapter::AdapterError) -> BrokerError {
    let hint = err.hint.unwrap_or_default();
    BrokerError::CdpIo(format!("cdp error {:?}: {}", err.kind, hint))
}

#[async_trait]
impl PermissionTransport for CdpPermissionTransport {
    async fn apply_permissions(
        &self,
        origin: &str,
        grant: &[String],
        revoke: &[String],
    ) -> Result<(), BrokerError> {
        if !grant.is_empty() {
            self.adapter
                .grant_permissions(origin, grant)
                .await
                .map_err(map_adapter_error)?;
        }

        if !revoke.is_empty() {
            self.adapter
                .reset_permissions(origin, revoke)
                .await
                .map_err(map_adapter_error)?;
        }

        Ok(())
    }
}
