use std::sync::Arc;

use async_trait::async_trait;
use soulbrowser_core_types::SoulError;

use crate::errors::SelectError;
use crate::model::{ActionReport, ExecCtx, SelectOpt, SelectParams};
use crate::policy::SelectPolicyView;
use crate::ports::{
    CdpPort, EventsPort, LocatorPort, MetricsPort, NetworkPort, StructPort, TempoPort,
};
use crate::runner::{execute, RuntimeDeps};

#[async_trait]
pub trait SelectTool: Send + Sync {
    async fn run(
        &self,
        ctx: ExecCtx,
        params: SelectParams,
        opt: SelectOpt,
    ) -> Result<ActionReport, SoulError>;
}

pub struct SelectToolBuilder {
    policy: SelectPolicyView,
    cdp: Option<Arc<dyn CdpPort>>,
    struct_port: Option<Arc<dyn StructPort>>,
    network: Option<Arc<dyn NetworkPort>>,
    locator: Option<Arc<dyn LocatorPort>>,
    events: Option<Arc<dyn EventsPort>>,
    metrics: Option<Arc<dyn MetricsPort>>,
    tempo: Option<Arc<dyn TempoPort>>,
}

impl SelectToolBuilder {
    pub fn new(policy: SelectPolicyView) -> Self {
        Self {
            policy,
            cdp: None,
            struct_port: None,
            network: None,
            locator: None,
            events: None,
            metrics: None,
            tempo: None,
        }
    }

    pub fn with_cdp(mut self, port: Arc<dyn CdpPort>) -> Self {
        self.cdp = Some(port);
        self
    }

    pub fn with_struct(mut self, port: Arc<dyn StructPort>) -> Self {
        self.struct_port = Some(port);
        self
    }

    pub fn with_network(mut self, port: Arc<dyn NetworkPort>) -> Self {
        self.network = Some(port);
        self
    }

    pub fn with_locator(mut self, port: Arc<dyn LocatorPort>) -> Self {
        self.locator = Some(port);
        self
    }

    pub fn with_events(mut self, port: Arc<dyn EventsPort>) -> Self {
        self.events = Some(port);
        self
    }

    pub fn with_metrics(mut self, port: Arc<dyn MetricsPort>) -> Self {
        self.metrics = Some(port);
        self
    }

    pub fn with_tempo(mut self, port: Arc<dyn TempoPort>) -> Self {
        self.tempo = Some(port);
        self
    }

    pub fn build(self) -> Arc<dyn SelectTool> {
        Arc::new(SelectToolImpl {
            policy: self.policy,
            cdp: self.cdp.expect("cdp port is required"),
            struct_port: self.struct_port.expect("struct port is required"),
            network: self.network.expect("network port is required"),
            locator: self.locator,
            events: self.events.expect("events port is required"),
            metrics: self.metrics.expect("metrics port is required"),
            tempo: self.tempo,
        })
    }
}

struct SelectToolImpl {
    policy: SelectPolicyView,
    cdp: Arc<dyn CdpPort>,
    struct_port: Arc<dyn StructPort>,
    network: Arc<dyn NetworkPort>,
    locator: Option<Arc<dyn LocatorPort>>,
    events: Arc<dyn EventsPort>,
    metrics: Arc<dyn MetricsPort>,
    tempo: Option<Arc<dyn TempoPort>>,
}

#[async_trait]
impl SelectTool for SelectToolImpl {
    async fn run(
        &self,
        ctx: ExecCtx,
        params: SelectParams,
        opt: SelectOpt,
    ) -> Result<ActionReport, SoulError> {
        if ctx.cancel.is_cancelled() {
            return Err(SelectError::Cancelled.into());
        }

        let runtime = RuntimeDeps {
            cdp: self.cdp.as_ref(),
            struct_port: self.struct_port.as_ref(),
            network: self.network.as_ref(),
            locator: self.locator.as_deref(),
            events: self.events.as_ref(),
            metrics: self.metrics.as_ref(),
            tempo: self.tempo.as_deref(),
            policy: &self.policy,
        };

        execute(&ctx, params, opt, runtime).await
    }
}
