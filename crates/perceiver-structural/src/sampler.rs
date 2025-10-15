use std::sync::Arc;

use soulbrowser_core_types::ExecRoute;

use crate::errors::PerceiverError;
use crate::model::{AnchorDescriptor, ResolveHint, SampledPair};
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

    pub async fn sample(&self, route: &ExecRoute) -> Result<SampledPair, PerceiverError> {
        self.port.sample_dom_ax(route).await
    }

    pub fn port(&self) -> Arc<P> {
        Arc::clone(&self.port)
    }

    pub async fn query(
        &self,
        route: &ExecRoute,
        hint: &ResolveHint,
    ) -> Result<Vec<AnchorDescriptor>, PerceiverError> {
        self.port.query(route, hint).await
    }
}
