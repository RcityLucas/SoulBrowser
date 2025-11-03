use async_trait::async_trait;
use perceiver_structural::AnchorDescriptor;
use soulbrowser_core_types::{ActionId, ExecRoute, SoulError};

use crate::model::{DomDigest, MatchKind, NetDigest, SelectMode, SelectParams, SelectionDigest};

#[async_trait]
pub trait CdpPort: Send + Sync {
    async fn scroll_into_view(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<(), SoulError>;
    async fn focus(&self, route: &ExecRoute, anchor: &AnchorDescriptor) -> Result<(), SoulError>;
    async fn select_option(
        &self,
        route: &ExecRoute,
        params: &SelectParams,
    ) -> Result<(), SoulError>;
    async fn wait_dom_ready(
        &self,
        route: &ExecRoute,
        timeout: std::time::Duration,
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
    async fn is_readonly(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<bool, SoulError>;
    async fn local_diff(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<DomDigest, SoulError>;
    async fn selection_state(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<SelectionState, SoulError>;
}

#[async_trait]
pub trait NetworkPort: Send + Sync {
    async fn window_digest(&self, route: &ExecRoute) -> Result<NetDigest, SoulError>;
}

#[async_trait]
pub trait LocatorPort: Send + Sync {
    async fn try_once(&self, req: HealRequest) -> Result<HealOutcome, SoulError>;
}

#[async_trait]
pub trait EventsPort: Send + Sync {
    async fn emit_started(&self, action: &ActionId, anchor: &AnchorDescriptor);
    async fn emit_precheck(&self, action: &ActionId, snapshot: &PrecheckEvent);
    async fn emit_finished(&self, action: &ActionId, signals: &PostEventPayload, ok: bool);
}

#[async_trait]
pub trait MetricsPort: Send + Sync {
    fn record_ok(&self, latency: u128);
    fn record_fail(&self, kind: &str);
    fn record_precheck_failure(&self, field: &str);
    fn record_self_heal(&self, success: bool);
    fn record_mode(&self, mode: &str);
    fn record_match_kind(&self, kind: &str);
}

#[async_trait]
pub trait TempoPort: Send + Sync {
    async fn plan(
        &self,
        route: &ExecRoute,
        control: &AnchorDescriptor,
        mode: SelectMode,
    ) -> Result<TempoPlan, SoulError>;
    async fn apply(&self, plan: &TempoPlan) -> Result<(), SoulError>;
}

#[derive(Clone, Debug, Default)]
pub struct TempoPlan {
    pub pre_delay_ms: u64,
    pub post_delay_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct SelectionState {
    pub selected_indices: Vec<u32>,
    pub selected_values: Vec<String>,
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
    pub readonly: bool,
}

#[derive(Clone, Debug)]
pub struct PostEventPayload {
    pub selection: SelectionDigest,
}

impl PostEventPayload {
    pub fn new(selection: SelectionDigest) -> Self {
        Self { selection }
    }
}

pub fn match_kind_label(kind: MatchKind) -> &'static str {
    match kind {
        MatchKind::Value => "value",
        MatchKind::Label => "label",
        MatchKind::Index => "index",
        MatchKind::Anchor => "anchor",
    }
}
