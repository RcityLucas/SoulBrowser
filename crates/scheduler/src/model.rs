use std::time::{Duration, Instant};

use soulbrowser_core_types::{ActionId, ExecRoute, RoutingHint, SoulError, ToolCall};
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Priority {
    Lightning,
    Quick,
    Standard,
    Deep,
}

impl Priority {
    pub const ALL: [Priority; 4] = [
        Priority::Lightning,
        Priority::Quick,
        Priority::Standard,
        Priority::Deep,
    ];

    pub fn weight(self) -> u8 {
        match self {
            Priority::Lightning => 8,
            Priority::Quick => 4,
            Priority::Standard => 2,
            Priority::Deep => 1,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Priority::Lightning => 0,
            Priority::Quick => 1,
            Priority::Standard => 2,
            Priority::Deep => 3,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CallOptions {
    pub timeout: Duration,
    pub priority: Priority,
    pub interruptible: bool,
    pub retry: RetryOpt,
}

impl Default for CallOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(8),
            priority: Priority::Standard,
            interruptible: true,
            retry: RetryOpt::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RetryOpt {
    pub max: u8,
    pub backoff: Duration,
}

impl Default for RetryOpt {
    fn default() -> Self {
        Self {
            max: 1,
            backoff: Duration::from_millis(300),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DispatchRequest {
    pub tool_call: ToolCall,
    pub options: CallOptions,
    pub routing_hint: Option<RoutingHint>,
}

#[derive(Clone, Debug)]
pub struct DispatchOutput {
    pub route: ExecRoute,
    pub error: Option<SoulError>,
    pub timeline: DispatchTimeline,
}

impl DispatchOutput {
    pub fn ok(route: ExecRoute, timeline: DispatchTimeline) -> Self {
        Self {
            route,
            error: None,
            timeline,
        }
    }

    pub fn err(route: ExecRoute, error: SoulError, timeline: DispatchTimeline) -> Self {
        Self {
            route,
            error: Some(error),
            timeline,
        }
    }
}

pub struct SubmitHandle {
    pub action_id: ActionId,
    pub receiver: oneshot::Receiver<DispatchOutput>,
}

#[derive(Clone, Debug)]
pub struct DispatchTimeline {
    pub enqueued_at: std::time::Instant,
    pub started_at: Option<std::time::Instant>,
    pub finished_at: Option<std::time::Instant>,
}

impl Default for DispatchTimeline {
    fn default() -> Self {
        Self {
            enqueued_at: Instant::now(),
            started_at: None,
            finished_at: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SchedulerConfig {
    pub global_slots: usize,
    pub per_task_limit: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            global_slots: 8,
            per_task_limit: 3,
        }
    }
}
