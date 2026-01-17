//! Core data types for action primitives

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use soulbrowser_core_types::ExecRoute;
use soulbrowser_policy_center::PolicyView;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::errors::ActionError;

/// Execution context for action primitives
///
/// Contains all the runtime context needed to execute an action:
/// - Route identifying the target frame
/// - Deadline for timeout enforcement
/// - Cancellation token for cooperative cancellation
/// - Policy view for authorization checks
/// - Unique action ID for tracing and correlation
#[derive(Clone)]
pub struct ExecCtx {
    /// Target execution route (session/page/frame)
    pub route: ExecRoute,

    /// Deadline for this operation
    pub deadline: Instant,

    /// Cancellation token for cooperative cancellation
    pub cancel_token: CancellationToken,

    /// Policy view for authorization checks
    pub policy_view: PolicyView,

    /// Unique identifier for this action
    pub action_id: String,
}

impl ExecCtx {
    /// Create a new execution context
    pub fn new(
        route: ExecRoute,
        deadline: Instant,
        cancel_token: CancellationToken,
        policy_view: PolicyView,
    ) -> Self {
        Self {
            route,
            deadline,
            cancel_token,
            policy_view,
            action_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    /// Check if this context has been cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    /// Check if this context has exceeded its deadline
    pub fn is_timeout(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Get remaining time until deadline
    pub fn remaining_time(&self) -> std::time::Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

/// Built-in waiting tiers for actions
///
/// Different actions have different default waiting strategies:
/// - None: No waiting (for explicit waits or when already stable)
/// - DomReady: Wait for DOM to be ready (quick actions)
/// - Idle: Wait for page to be idle with network quiet (navigation)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaitTier {
    /// No built-in waiting
    None,

    /// Wait for DOM ready event
    DomReady,

    /// Wait for page idle (DOM ready + 500ms network quiet)
    Idle,
}

impl Default for WaitTier {
    fn default() -> Self {
        WaitTier::DomReady
    }
}

/// Pre-check result before executing an action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrecheckResult {
    /// Whether precheck passed
    pub passed: bool,

    /// Reason for failure (if failed)
    pub reason: Option<String>,

    /// Timestamp of precheck
    pub checked_at: DateTime<Utc>,
}

/// Post-action signals captured after execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostSignals {
    /// DOM mutation count during action
    pub dom_mutations: u32,

    /// Network requests initiated during action
    pub network_requests: u32,

    /// Console messages logged during action
    pub console_messages: Vec<String>,

    /// URL after action (if changed)
    pub url_after: Option<String>,

    /// Title after action (if changed)
    pub title_after: Option<String>,
}

impl Default for PostSignals {
    fn default() -> Self {
        Self {
            dom_mutations: 0,
            network_requests: 0,
            console_messages: Vec::new(),
            url_after: None,
            title_after: None,
        }
    }
}

/// Self-heal information (populated by locator if heal occurred)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfHealInfo {
    /// Original anchor that failed
    pub original_anchor: String,

    /// New anchor that succeeded
    pub healed_anchor: String,

    /// Strategy used for healing (css/aria/text)
    pub strategy: String,

    /// Confidence score of the heal (0.0-1.0)
    pub confidence: f64,
}

/// Comprehensive action execution report
///
/// Contains all information about an action's execution:
/// - Success/failure status
/// - Timing information
/// - Pre-check results
/// - Post-execution signals
/// - Self-heal information (if applicable)
/// - Error details (if failed)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionReport {
    /// Whether the action succeeded
    pub ok: bool,

    /// When the action started
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub started_at: DateTime<Utc>,

    /// When the action finished
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub finished_at: DateTime<Utc>,

    /// Total latency in milliseconds
    pub latency_ms: u64,

    /// Pre-check result (if applicable)
    pub precheck: Option<PrecheckResult>,

    /// Post-execution signals
    pub post_signals: PostSignals,

    /// Self-heal information (if healing occurred)
    pub self_heal: Option<SelfHealInfo>,

    /// Error details (if failed)
    pub error: Option<String>,
}

impl ActionReport {
    /// Create a successful action report
    pub fn success(started_at: DateTime<Utc>, latency_ms: u64) -> Self {
        Self {
            ok: true,
            started_at,
            finished_at: Utc::now(),
            latency_ms,
            precheck: None,
            post_signals: PostSignals::default(),
            self_heal: None,
            error: None,
        }
    }

    /// Create a failed action report
    pub fn failure(started_at: DateTime<Utc>, latency_ms: u64, error: ActionError) -> Self {
        Self {
            ok: false,
            started_at,
            finished_at: Utc::now(),
            latency_ms,
            precheck: None,
            post_signals: PostSignals::default(),
            self_heal: None,
            error: Some(error.to_string()),
        }
    }

    /// Add precheck result
    pub fn with_precheck(mut self, precheck: PrecheckResult) -> Self {
        self.precheck = Some(precheck);
        self
    }

    /// Add post signals
    pub fn with_signals(mut self, signals: PostSignals) -> Self {
        self.post_signals = signals;
        self
    }

    /// Add self-heal information
    pub fn with_heal(mut self, heal: SelfHealInfo) -> Self {
        self.self_heal = Some(heal);
        self
    }
}

/// Anchor descriptor for element targeting
///
/// Represents different strategies for locating elements:
/// - CSS selector
/// - ARIA/AX attributes (role + name)
/// - Text content matching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AnchorDescriptor {
    /// CSS selector
    Css(String),

    /// ARIA/AX role and accessible name
    Aria { role: String, name: String },

    /// Text content (exact or partial match)
    Text { content: String, exact: bool },
}

impl AnchorDescriptor {
    /// Convert to string representation for logging
    pub fn to_string(&self) -> String {
        match self {
            AnchorDescriptor::Css(s) => format!("css:{}", s),
            AnchorDescriptor::Aria { role, name } => {
                format!("aria:{}[name='{}']", role, name)
            }
            AnchorDescriptor::Text { content, exact } => {
                if *exact {
                    format!("text:exact:'{}'", content)
                } else {
                    format!("text:partial:'{}'", content)
                }
            }
        }
    }
}

/// Scroll target specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScrollTarget {
    /// Scroll to top of page
    Top,

    /// Scroll to bottom of page
    Bottom,

    /// Scroll to specific element
    Element(AnchorDescriptor),

    /// Scroll by pixel amount (positive=down, negative=up)
    Pixels(i32),
}

/// Scroll behavior (smooth vs instant)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollBehavior {
    /// Smooth animated scroll
    Smooth,

    /// Instant jump to position
    Instant,
}

impl Default for ScrollBehavior {
    fn default() -> Self {
        ScrollBehavior::Smooth
    }
}

/// Select method for dropdown/listbox selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectMethod {
    /// Select by visible text
    Text,

    /// Select by value attribute
    Value,

    /// Select by index (0-based)
    Index,
}

impl Default for SelectMethod {
    fn default() -> Self {
        SelectMethod::Value
    }
}

/// Wait condition for explicit waits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WaitCondition {
    /// Wait for element to be visible
    ElementVisible(AnchorDescriptor),

    /// Wait for element to be hidden
    ElementHidden(AnchorDescriptor),

    /// Wait for URL to satisfy regex/contains
    UrlMatches(String),

    /// Wait for URL to equal exact string
    UrlEquals(String),

    /// Wait for title to match pattern
    TitleMatches(String),

    /// Wait for fixed duration (milliseconds)
    Duration(u64),

    /// Wait for network to be idle (no requests for N ms)
    NetworkIdle(u64),
}
