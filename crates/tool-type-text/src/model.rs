use std::time::{Duration, Instant};

use bitflags::bitflags;
use perceiver_structural::AnchorDescriptor;
use serde::{Deserialize, Serialize};
use soulbrowser_core_types::{ActionId, ExecRoute};
use tokio_util::sync::CancellationToken;

/// Execution context delivered by the scheduler when invoking the tool.
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

/// Controls how text is injected.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum InputMode {
    Character,
    Instant,
    Natural,
    Paste,
}

impl Default for InputMode {
    fn default() -> Self {
        InputMode::Character
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct ClearMethod: u8 {
        const NONE = 0b0000;
        const SELECT_ALL_DELETE = 0b0001;
        const BACKSPACE = 0b0010;
    }
}

impl Default for ClearMethod {
    fn default() -> Self {
        ClearMethod::SELECT_ALL_DELETE
    }
}

/// Parameters describing the intended text entry.
#[derive(Clone, Debug)]
pub struct TextParams {
    pub anchor: AnchorDescriptor,
    pub text: String,
    pub mode: InputMode,
    pub clear: ClearConfig,
    pub submit: bool,
}

impl TextParams {
    pub fn new(anchor: AnchorDescriptor, text: String) -> Self {
        Self {
            anchor,
            text,
            mode: InputMode::Character,
            clear: ClearConfig::default(),
            submit: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ClearConfig {
    pub enabled: bool,
    pub method: ClearMethod,
    pub max_backspace: u32,
}

impl Default for ClearConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            method: ClearMethod::SELECT_ALL_DELETE,
            max_backspace: 64,
        }
    }
}

/// Desired waiting behaviour post input.
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

/// Optional runtime tuning knobs.
#[derive(Clone, Debug, Default)]
pub struct TextOpt {
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
    pub maxlength: Option<u32>,
    pub password_like: bool,
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
    pub value: ValueDigest,
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
pub struct ValueDigest {
    pub changed: bool,
    pub old_len: Option<usize>,
    pub new_len: Option<usize>,
    pub hash_after: Option<String>,
}

pub fn remaining_deadline(ctx: &ExecCtx) -> Duration {
    ctx.deadline
        .checked_duration_since(Instant::now())
        .unwrap_or_else(|| Duration::from_secs(0))
}
