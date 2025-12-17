use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::auth::BrowserAuthManager;
use crate::errors::SoulBrowserError;
use crate::storage::StorageManager;
use crate::tools::BrowserToolManager;

/// Abstraction over external integrations (auth, storage, tools) so that
/// different environments can provide their own implementations.
#[async_trait]
pub trait IntegrationProvider: Send + Sync {
    async fn create_storage_manager(
        &self,
        storage_path: Option<PathBuf>,
    ) -> Result<Arc<StorageManager>, SoulBrowserError>;

    async fn create_auth_manager(
        &self,
        tenant_id: String,
        policy_paths: &[PathBuf],
    ) -> Result<Arc<BrowserAuthManager>, SoulBrowserError>;

    async fn create_tool_manager(
        &self,
        tenant_id: String,
    ) -> Result<Arc<BrowserToolManager>, SoulBrowserError>;
}

/// Default provider backed by the existing soul-base implementations.
#[derive(Default)]
pub struct SoulbaseIntegrationProvider;

#[async_trait]
impl IntegrationProvider for SoulbaseIntegrationProvider {
    async fn create_storage_manager(
        &self,
        storage_path: Option<PathBuf>,
    ) -> Result<Arc<StorageManager>, SoulBrowserError> {
        let storage = Arc::new(match storage_path {
            Some(path) => StorageManager::file_based(path),
            None => StorageManager::in_memory(),
        });
        Ok(storage)
    }

    async fn create_auth_manager(
        &self,
        tenant_id: String,
        policy_paths: &[PathBuf],
    ) -> Result<Arc<BrowserAuthManager>, SoulBrowserError> {
        let manager = if policy_paths.is_empty() {
            BrowserAuthManager::new(tenant_id.clone()).await
        } else {
            BrowserAuthManager::with_policy_paths(tenant_id.clone(), policy_paths).await
        }
        .map_err(|err| {
            SoulBrowserError::internal(&format!("Failed to create auth manager: {}", err))
        })?;
        Ok(Arc::new(manager))
    }

    async fn create_tool_manager(
        &self,
        tenant_id: String,
    ) -> Result<Arc<BrowserToolManager>, SoulBrowserError> {
        let manager = Arc::new(BrowserToolManager::new(tenant_id));
        manager.register_default_tools().await.map_err(|err| {
            SoulBrowserError::internal(&format!("Failed to register tools: {}", err))
        })?;
        Ok(manager)
    }
}

/// Helper to construct the default integration provider based on the active
/// feature flags.
pub fn default_provider() -> Arc<dyn IntegrationProvider> {
    Arc::new(SoulbaseIntegrationProvider::default())
}
