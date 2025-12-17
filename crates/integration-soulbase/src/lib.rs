//! Soul-base integration provider implementation.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use soulbrowser_kernel::auth::BrowserAuthManager;
use soulbrowser_kernel::errors::SoulBrowserError;
use soulbrowser_kernel::integration::IntegrationProvider;
use soulbrowser_kernel::storage::StorageManager;
use soulbrowser_kernel::tools::BrowserToolManager;

/// Integration provider backed by soul-base crates.
pub struct SoulbaseProvider;

impl Default for SoulbaseProvider {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl IntegrationProvider for SoulbaseProvider {
    async fn create_storage_manager(
        &self,
        storage_path: Option<PathBuf>,
    ) -> Result<Arc<StorageManager>, SoulBrowserError> {
        let manager = match storage_path {
            Some(path) => StorageManager::file_based(path),
            None => StorageManager::in_memory(),
        };
        Ok(Arc::new(manager))
    }

    async fn create_auth_manager(
        &self,
        tenant_id: String,
        policy_paths: &[PathBuf],
    ) -> Result<Arc<BrowserAuthManager>, SoulBrowserError> {
        let auth = if policy_paths.is_empty() {
            BrowserAuthManager::new(tenant_id.clone()).await
        } else {
            BrowserAuthManager::with_policy_paths(tenant_id.clone(), policy_paths).await
        }
        .map_err(|err| {
            SoulBrowserError::internal(&format!("Failed to create auth manager: {}", err))
        })?;
        Ok(Arc::new(auth))
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
