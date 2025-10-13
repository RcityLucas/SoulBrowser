use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::target::SetDiscoverTargetsParams;
use chromiumoxide::cdp::browser_protocol::target::{AttachToTargetParams, TargetInfo};
use chromiumoxide::handler::HandlerConfig;
use serde_json::json;
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, error, info};

use crate::config::CdpConfig;
use crate::events::RawEvent;
use crate::ids::{BrowserId, FrameId, PageId, SessionId};
use crate::registry::Registry;
use crate::transport::{CdpTransport, CommandTarget, NoopTransport};
use crate::wait::WaitGate;
use crate::{error::AdapterError, wait::gate::GateWaiter};

#[async_trait]
pub trait CdpAdapter: Send + Sync {
    async fn navigate(&self, page: PageId, url: &str, timeout: Duration) -> Result<(), AdapterError>;
}

pub struct ChromiumCdpAdapter {
    browser: Browser,
    transport: Arc<dyn CdpTransport>,
    registry: Arc<Registry>,
    events: broadcast::Sender<RawEvent>,
}

impl ChromiumCdpAdapter {
    pub async fn new(config: CdpConfig) -> Result<Self> {
        let handler = HandlerConfig::builder().build();
        let browser_config = BrowserConfig::builder()
            .with_headless(config.headless)
            .with_path(config.executable)
            .with_user_data_dir(config.user_data_dir)
            .build()?;
        let (browser, _handler) = Browser::launch(browser_config).await?;
        let transport: Arc<dyn CdpTransport> = Arc::new(NoopTransport::default());
        let registry = Arc::new(Registry::new());
        let (tx, _) = broadcast::channel(512);
        Ok(Self {
            browser,
            transport,
            registry,
            events: tx,
        })
    }

    pub fn registry(&self) -> Arc<Registry> {
        Arc::clone(&self.registry)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RawEvent> {
        self.events.subscribe()
    }
}

#[async_trait]
impl CdpAdapter for ChromiumCdpAdapter {
    async fn navigate(&self, _page: PageId, url: &str, timeout: Duration) -> Result<(), AdapterError> {
        self.transport
            .send_command(CommandTarget::Browser, "Page.navigate", json!({ "url": url }))
            .await?
            ;
        GateWaiter::new(WaitGate::NetworkQuiet, timeout)
            .await
            .map_err(|_| AdapterError::new("timeout".into()))
    }
}
