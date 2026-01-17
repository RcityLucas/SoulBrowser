use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use tokio::sync::OnceCell;

use crate::cli::telemetry::load_persistent_sinks;
use soulbrowser_kernel::app_context::{create_context, get_or_create_context, AppContext};
use soulbrowser_kernel::tool_registry::ToolRegistry;
use soulbrowser_kernel::Config;

const DEFAULT_TENANT_ID: &str = "cli";

pub struct CliContext {
    config: Arc<Config>,
    config_path: PathBuf,
    metrics_port: u16,
    app_context: OnceCell<Arc<AppContext>>,
    tool_registry_loaded: OnceCell<()>,
    telemetry_loaded: OnceCell<()>,
}

impl CliContext {
    pub fn new(config: Config, config_path: PathBuf, metrics_port: u16) -> Self {
        Self {
            config: Arc::new(config),
            config_path,
            metrics_port,
            app_context: OnceCell::new(),
            tool_registry_loaded: OnceCell::new(),
            telemetry_loaded: OnceCell::new(),
        }
    }

    pub fn config(&self) -> &Config {
        self.config.as_ref()
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn config_dir(&self) -> PathBuf {
        self.config_path
            .parent()
            .map(|path| path.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn metrics_port(&self) -> u16 {
        self.metrics_port
    }

    pub async fn app_context(&self) -> Result<Arc<AppContext>> {
        self.app_context
            .get_or_try_init(|| async {
                get_or_create_context(
                    DEFAULT_TENANT_ID.to_string(),
                    Some(self.config.output_dir.clone()),
                    self.config.policy_paths.clone(),
                )
                .await
                .map_err(|err| err.into())
            })
            .await
            .map(Arc::clone)
    }

    pub async fn app_context_with(
        &self,
        tenant: &str,
        storage_path: Option<PathBuf>,
    ) -> Result<Arc<AppContext>> {
        let normalized_storage = storage_path.or_else(|| Some(self.config.output_dir.clone()));
        let is_default_tenant = tenant == DEFAULT_TENANT_ID;
        let is_default_storage = normalized_storage
            .as_ref()
            .map(|path| path == &self.config.output_dir)
            .unwrap_or(false);

        if is_default_tenant && is_default_storage {
            return self.app_context().await;
        }

        create_context(
            tenant.to_string(),
            normalized_storage,
            self.config.policy_paths.clone(),
        )
        .await
        .map_err(|err| err.into())
    }
}

impl CliContext {
    pub async fn ensure_tool_registry_loaded(&self, registry: Arc<ToolRegistry>) -> Result<()> {
        let config_dir = self
            .config_path
            .parent()
            .map(|path| path.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let tools_dir = config_dir.join("tools");
        let dir_clone = tools_dir.clone();
        let registry_clone = registry.clone();
        self.tool_registry_loaded
            .get_or_try_init(|| {
                let dir = dir_clone.clone();
                let registry = registry_clone.clone();
                async move {
                    if dir.exists() {
                        registry.load_from_dir(&dir).map_err(|err| anyhow!(err))?;
                    }
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await?;
        Ok(())
    }

    pub async fn init_telemetry(&self) -> Result<()> {
        let dir = self.config_dir();
        self.telemetry_loaded
            .get_or_try_init(|| async { load_persistent_sinks(&dir) })
            .await?;
        Ok(())
    }
}
