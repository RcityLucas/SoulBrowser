use std::time::{Duration, Instant};

use bitflags::bitflags;
use perceiver_structural::AnchorDescriptor;
use soulbrowser_core_types::{ActionId, ExecRoute, SoulError};
use tokio_util::sync::CancellationToken;

/// Execution context delivered by the scheduler.
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

/// Mouse button selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseBtn {
    Left,
    Middle,
    Right,
}

impl Default for MouseBtn {
    fn default() -> Self {
        MouseBtn::Left
    }
}

bitflags! {
    pub struct KeyMod: u8 {
        const CTRL = 0b0001;
        const SHIFT = 0b0010;
        const ALT = 0b0100;
        const META = 0b1000;
    }
}

impl Default for KeyMod {
    fn default() -> Self {
        KeyMod::empty()
    }
}

/// Parameters for executing a click.
#[derive(Clone, Debug)]
pub struct ClickParams {
    pub anchor: AnchorDescriptor,
    pub button: MouseBtn,
    pub modifiers: KeyMod,
    pub click_count: u8,
    pub offset: Option<(i32, i32)>,
}

/// Wait strategy after dispatching the click.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

/// Optional execution tweaks.
#[derive(Clone, Debug, Default)]
pub struct ClickOpt {
    pub wait: WaitTier,
    pub timeout_ms: Option<u64>,
    pub priority: Option<u8>,
}

/// Outcome of the click execution.
#[derive(Clone, Debug)]
pub struct ActionReport {
    pub ok: bool,
    pub started_at: Instant,
    pub finished_at: Instant,
    pub latency_ms: u128,
    pub precheck: Option<PrecheckSnapshot>,
    pub post_signals: PostSignals,
    pub self_heal: Option<SelfHeal>,
    pub error: Option<SoulError>,
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

#[derive(Clone, Debug)]
pub struct PrecheckSnapshot {
    pub visible: bool,
    pub clickable: bool,
    pub enabled: Option<bool>,
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

impl PostSignals {
    pub fn merge(
        dom: DomDigest,
        net: NetDigest,
        url: Option<String>,
        title: Option<String>,
    ) -> Self {
        Self {
            dom,
            net,
            url,
            title,
        }
    }
}

/// Helper to convert relative waits to absolute deadlines.
pub fn remaining_deadline(ctx: &ExecCtx) -> Duration {
    ctx.deadline
        .checked_duration_since(Instant::now())
        .unwrap_or_else(|| Duration::from_secs(0))
}
