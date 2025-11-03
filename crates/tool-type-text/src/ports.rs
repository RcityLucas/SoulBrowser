use async_trait::async_trait;
use perceiver_structural::AnchorDescriptor;
use soulbrowser_core_types::{ActionId, ExecRoute, SoulError};

use crate::model::{DomDigest, InputMode, NetDigest, ValueDigest};

#[async_trait]
pub trait CdpPort: Send + Sync {
    async fn scroll_into_view(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<(), SoulError>;
    async fn focus(&self, route: &ExecRoute, anchor: &AnchorDescriptor) -> Result<(), SoulError>;
    async fn clear_select_all(&self, route: &ExecRoute) -> Result<(), SoulError>;
    async fn clear_backspace(&self, route: &ExecRoute, limit: u32) -> Result<(), SoulError>;
    async fn keyboard_type(&self, route: &ExecRoute, text: &str) -> Result<(), SoulError>;
    async fn insert_text(&self, route: &ExecRoute, text: &str) -> Result<(), SoulError>;
    async fn paste_text(&self, route: &ExecRoute, text: &str) -> Result<(), SoulError>;
    async fn key_submit(&self, route: &ExecRoute) -> Result<(), SoulError>;
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
    async fn field_meta(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<FieldMeta, SoulError>;
    async fn local_diff(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<DomDigest, SoulError>;
}

#[async_trait]
pub trait NetworkPort: Send + Sync {
    async fn window_digest(&self, route: &ExecRoute) -> Result<NetDigest, SoulError>;
    async fn value_digest(
        &self,
        route: &ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<ValueDigest, SoulError>;
}

#[async_trait]
pub trait LocatorPort: Send + Sync {
    async fn try_once(&self, req: HealRequest) -> Result<HealOutcome, SoulError>;
}

#[async_trait]
pub trait EventsPort: Send + Sync {
    async fn emit_started(&self, action: &ActionId, anchor: &AnchorDescriptor);
    async fn emit_precheck(&self, action: &ActionId, snapshot: &PrecheckEvent);
    async fn emit_finished(&self, action: &ActionId, report: &ValueDigest, ok: bool);
}

#[async_trait]
pub trait MetricsPort: Send + Sync {
    fn record_ok(&self, latency: u128);
    fn record_fail(&self, kind: &str);
    fn record_precheck_failure(&self, field: &str);
    fn record_self_heal(&self, success: bool);
    fn record_mode(&self, mode: &str);
}

#[async_trait]
pub trait TempoPort: Send + Sync {
    async fn build_plan(&self, mode: InputMode, text: &str) -> Result<TypingPlan, SoulError>;
    async fn run_plan(&self, route: &ExecRoute, plan: &TypingPlan) -> Result<(), SoulError>;
}

#[derive(Clone, Debug, Default)]
pub struct TypingPlan {
    pub steps: Vec<TypingStep>,
}

#[derive(Clone, Debug)]
pub struct TypingStep {
    pub chunk: String,
    pub delay_ms: u64,
}

#[derive(Clone, Debug)]
pub struct FieldMeta {
    pub readonly: bool,
    pub maxlength: Option<u32>,
    pub password_like: bool,
}

#[derive(Clone, Debug)]
pub struct HealRequest {
    pub action_id: ActionId,
    pub route: ExecRoute,
    pub primary: AnchorDescriptor,
    pub reason: String,
}

pub struct HealOutcome {
    pub used_anchor: Option<AnchorDescriptor>,
}

impl HealOutcome {
    pub fn new(anchor: Option<AnchorDescriptor>) -> Self {
        Self {
            used_anchor: anchor,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PrecheckEvent {
    pub visible: bool,
    pub clickable: bool,
    pub enabled: Option<bool>,
    pub readonly: bool,
}
