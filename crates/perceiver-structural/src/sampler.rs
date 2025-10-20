use std::sync::Arc;

use serde_json::Value;
use soulbrowser_core_types::ExecRoute;
use tokio::time::{sleep, Duration};

use crate::errors::PerceiverError;
use crate::model::{AnchorDescriptor, ResolveHint, SampledPair, Scope, SnapLevel};
use crate::ports::CdpPerceptionPort;

pub struct Sampler<P>
where
    P: CdpPerceptionPort + Send + Sync,
{
    port: Arc<P>,
}

impl<P> Sampler<P>
where
    P: CdpPerceptionPort + Send + Sync,
{
    pub fn new(port: Arc<P>) -> Self {
        Self { port }
    }

    pub async fn sample(
        &self,
        route: &ExecRoute,
        scope: &Scope,
        level: SnapLevel,
    ) -> Result<SampledPair, PerceiverError> {
        const MAX_ATTEMPTS: usize = 3;
        const BACKOFF_MS: u64 = 50;
        let mut attempt = 0;
        let mut last_err: Option<PerceiverError> = None;
        while attempt < MAX_ATTEMPTS {
            match self.port.sample_dom_ax(route, scope, level).await {
                Ok(pair) => return Ok(pair),
                Err(err) => {
                    last_err = Some(err);
                    attempt += 1;
                    if attempt < MAX_ATTEMPTS {
                        let backoff = BACKOFF_MS * (attempt as u64);
                        sleep(Duration::from_millis(backoff)).await;
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| PerceiverError::internal("sampler failed")))
    }

    pub fn port(&self) -> Arc<P> {
        Arc::clone(&self.port)
    }

    pub async fn query(
        &self,
        route: &ExecRoute,
        hint: &ResolveHint,
        scope: &Scope,
    ) -> Result<Vec<AnchorDescriptor>, PerceiverError> {
        self.port.query(route, hint, scope).await
    }

    pub async fn describe_backend_node(
        &self,
        route: &ExecRoute,
        backend_node_id: u64,
    ) -> Result<Value, PerceiverError> {
        self.port
            .describe_backend_node(route, backend_node_id)
            .await
    }

    pub async fn node_attributes(
        &self,
        route: &ExecRoute,
        backend_node_id: u64,
    ) -> Result<Option<Value>, PerceiverError> {
        self.port.node_attributes(route, backend_node_id).await
    }

    pub async fn node_style(
        &self,
        route: &ExecRoute,
        backend_node_id: u64,
    ) -> Result<Option<Value>, PerceiverError> {
        self.port.node_style(route, backend_node_id).await
    }
}
