use async_trait::async_trait;
use chrono::DateTime;
use perceiver_structural::AnchorDescriptor;
use soulbrowser_core_types::{ActionId, ExecRoute, SoulError};

use crate::model::{DomDigest, MouseBtn, NetDigest, PostSignals};

#[async_trait]
pub trait CdpPort: Send + Sync {
    async fn scroll_into_view(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<(), SoulError>;
    async fn focus(&self, route: &ExecRoute, anchor: &AnchorDescriptor) -> Result<(), SoulError>;
    async fn element_center(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<(i32, i32), SoulError>;
    async fn dispatch_click(
        &self,
        route: &ExecRoute,
        coords: (i32, i32),
        button: MouseBtn,
        click_count: u8,
        modifiers: u8,
    ) -> Result<(), SoulError>;
    async fn wait_dom_ready(
        &self,
        route: &ExecRoute,
        until: DateTime<chrono::Utc>,
    ) -> Result<(), SoulError>;
    async fn current_url(&self, route: &ExecRoute) -> Result<String, SoulError>;
    async fn current_title(&self, route: &ExecRoute) -> Result<String, SoulError>;
}

#[async_trait]
pub trait StructPort: Send + Sync {
    async fn is_visible(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<bool, SoulError>;
    async fn is_clickable(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<bool, SoulError>;
    async fn is_enabled(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<Option<bool>, SoulError>;
    async fn local_diff(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<DomDigest, SoulError>;
}

#[async_trait]
pub trait NetworkPort: Send + Sync {
    async fn window_digest(&self, route: &ExecRoute) -> Result<NetDigest, SoulError>;
}

#[async_trait]
pub trait LocatorPort: Send + Sync {
    async fn try_once(&self, ctx: HealRequest) -> Result<HealOutcome, SoulError>;
}

#[async_trait]
pub trait EventsPort: Send + Sync {
    async fn emit_started(&self, action: &ActionId, anchor: &AnchorDescriptor);
    async fn emit_precheck(&self, action: &ActionId, precheck: &PrecheckEvent);
    async fn emit_finished(
        &self,
        action: &ActionId,
        report: &PostSignals,
        ok: bool,
        error: Option<&SoulError>,
    );
}

#[async_trait]
pub trait MetricsPort: Send + Sync {
    fn record_ok(&self, latency: u128);
    fn record_fail(&self, kind: &str);
    fn record_precheck_failure(&self, field: &str);
    fn record_self_heal(&self, success: bool);
}

#[async_trait]
pub trait TempoPort: Send + Sync {
    async fn prepare(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<TempoPlan, SoulError>;
    async fn apply(&self, plan: &TempoPlan) -> Result<(), SoulError>;
}

#[derive(Clone, Debug)]
pub struct HealRequest {
    pub action_id: ActionId,
    pub route: ExecRoute,
    pub primary: AnchorDescriptor,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct HealOutcome {
    pub used_anchor: Option<AnchorDescriptor>,
}

#[derive(Clone, Debug, Default)]
pub struct PrecheckEvent {
    pub visible: bool,
    pub clickable: bool,
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct TempoPlan {
    pub hover_ms: u64,
    pub dwell_ms: u64,
}
