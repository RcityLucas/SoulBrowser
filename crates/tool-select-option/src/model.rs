use std::time::{Duration, Instant};

use perceiver_structural::AnchorDescriptor;
use serde::{Deserialize, Serialize};
use soulbrowser_core_types::{ActionId, ExecRoute};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct ExecCtx {
    pub action_id: ActionId,
    pub route: ExecRoute,
    pub deadline: Instant,
    pub cancel: CancellationToken,
}

impl ExecCtx {
    pub fn new(
        action_id: ActionId,
        route: ExecRoute,
        deadline: Instant,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            action_id,
            route,
            deadline,
            cancel,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MatchKind {
    Value,
    Label,
    Index,
    Anchor,
}

impl Default for MatchKind {
    fn default() -> Self {
        MatchKind::Value
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SelectMode {
    Single,
    Multiple,
    Toggle,
}

impl Default for SelectMode {
    fn default() -> Self {
        SelectMode::Single
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum WaitTier {
    Auto,
    DomReady,
    None,
}

impl Default for WaitTier {
    fn default() -> Self {
        WaitTier::Auto
    }
}

#[derive(Clone, Debug)]
pub struct SelectParams {
    pub control_anchor: AnchorDescriptor,
    pub match_kind: MatchKind,
    pub item: String,
    pub option_anchor: Option<AnchorDescriptor>,
    pub mode: SelectMode,
}

#[derive(Clone, Debug, Default)]
pub struct SelectOpt {
    pub wait: WaitTier,
    pub timeout_ms: Option<u64>,
    pub priority: Option<u8>,
}

#[derive(Clone, Debug)]
pub struct ActionReport {
    pub ok: bool,
    pub started_at: Instant,
    pub finished_at: Instant,
    pub latency_ms: u128,
    pub precheck: Option<FieldSnapshot>,
    pub post_signals: PostSignals,
    pub self_heal: Option<SelfHeal>,
    pub error: Option<String>,
}

impl ActionReport {
    pub fn new(started_at: Instant) -> Self {
        Self {
            ok: false,
            started_at,
            finished_at: started_at,
            latency_ms: 0,
            precheck: None,
            post_signals: PostSignals::default(),
            self_heal: None,
            error: None,
        }
    }

    pub fn finish(mut self, finished_at: Instant) -> Self {
        self.finished_at = finished_at;
        self.latency_ms = finished_at
            .saturating_duration_since(self.started_at)
            .as_millis();
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct FieldSnapshot {
    pub visible: bool,
    pub clickable: bool,
    pub enabled: Option<bool>,
    pub readonly: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SelfHeal {
    pub attempted: bool,
    pub reason: Option<String>,
    pub used_anchor: Option<AnchorDescriptor>,
}

#[derive(Clone, Debug, Default)]
pub struct PostSignals {
    pub dom: DomDigest,
    pub net: NetDigest,
    pub selection: SelectionDigest,
    pub url: Option<String>,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DomDigest {
    pub changed_nodes: u32,
    pub focus_changed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct NetDigest {
    pub res2xx: u32,
    pub redirects: u32,
}

#[derive(Clone, Debug, Default)]
pub struct SelectionDigest {
    pub changed: bool,
    pub selected_count: usize,
    pub selected_indices: Vec<u32>,
    pub selected_hash: Option<String>,
}

pub fn remaining_deadline(ctx: &ExecCtx) -> Duration {
    ctx.deadline
        .checked_duration_since(Instant::now())
        .unwrap_or_else(|| Duration::from_secs(0))
}
