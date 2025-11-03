//! Core types for flow orchestration

use action_gate::ExpectSpec;
use action_primitives::{ActionReport, AnchorDescriptor};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Flow definition - orchestrates multiple action steps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    /// Flow identifier
    pub id: String,

    /// Flow name
    pub name: String,

    /// Flow description
    pub description: String,

    /// Root node of the flow
    pub root: FlowNode,

    /// Flow-level timeout in milliseconds
    pub timeout_ms: u64,

    /// Default failure strategy for steps
    pub default_failure_strategy: FailureStrategy,

    /// Flow metadata
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Flow {
    /// Create a new flow
    pub fn new(id: String, name: String, root: FlowNode) -> Self {
        Self {
            id,
            name,
            description: String::new(),
            root,
            timeout_ms: 300_000, // 5 minutes default
            default_failure_strategy: FailureStrategy::Abort,
            metadata: HashMap::new(),
        }
    }

    /// Set description
    pub fn with_description(mut self, description: String) -> Self {
        self.description = description;
        self
    }

    /// Set timeout
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Set default failure strategy
    pub fn with_default_strategy(mut self, strategy: FailureStrategy) -> Self {
        self.default_failure_strategy = strategy;
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: String, value: serde_json::Value) -> Self {
        self.metadata.insert(key, value);
        self
    }
}

/// Flow node - represents a step or control structure in the flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlowNode {
    /// Execute steps in sequence
    Sequence { steps: Vec<FlowNode> },

    /// Execute steps in parallel
    Parallel {
        steps: Vec<FlowNode>,
        /// Wait for all to complete (true) or first success (false)
        wait_all: bool,
    },

    /// Conditional execution
    Conditional {
        /// Condition to evaluate
        condition: FlowCondition,
        /// Execute if condition is true
        then_branch: Box<FlowNode>,
        /// Execute if condition is false (optional)
        else_branch: Option<Box<FlowNode>>,
    },

    /// Loop execution
    Loop {
        /// Body to execute
        body: Box<FlowNode>,
        /// Loop condition
        condition: LoopCondition,
        /// Maximum iterations
        max_iterations: u32,
    },

    /// Single action step
    Action {
        /// Step identifier
        id: String,
        /// Action type
        action: ActionType,
        /// Post-conditions to validate
        expect: Option<ExpectSpec>,
        /// Failure strategy for this step
        failure_strategy: Option<FailureStrategy>,
    },
}

/// Flow condition types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlowCondition {
    /// Element exists
    ElementExists(AnchorDescriptor),

    /// Element visible
    ElementVisible(AnchorDescriptor),

    /// URL matches pattern
    UrlMatches(String),

    /// Title matches pattern
    TitleMatches(String),

    /// JavaScript expression evaluates to true
    JsEvaluates(String),

    /// Previous step succeeded
    PreviousStepSucceeded,

    /// Variable equals value
    VariableEquals {
        name: String,
        value: serde_json::Value,
    },

    /// AND combination
    And(Vec<FlowCondition>),

    /// OR combination
    Or(Vec<FlowCondition>),

    /// NOT
    Not(Box<FlowCondition>),
}

/// Loop condition types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoopCondition {
    /// Loop while condition is true
    While(FlowCondition),

    /// Loop until condition is true
    Until(FlowCondition),

    /// Loop fixed number of times
    Count(u32),

    /// Loop forever (until break or error)
    Infinite,
}

/// Action types that can be executed in flows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionType {
    /// Navigate to URL
    Navigate {
        url: String,
        wait_tier: action_primitives::WaitTier,
    },

    /// Click element
    Click {
        anchor: AnchorDescriptor,
        wait_tier: action_primitives::WaitTier,
    },

    /// Type text
    TypeText {
        anchor: AnchorDescriptor,
        text: String,
        submit: bool,
        wait_tier: action_primitives::WaitTier,
    },

    /// Select option
    Select {
        anchor: AnchorDescriptor,
        option: String,
        method: Option<action_primitives::SelectMethod>,
        wait_tier: Option<action_primitives::WaitTier>,
    },

    /// Scroll
    Scroll {
        target: action_primitives::ScrollTarget,
        behavior: action_primitives::ScrollBehavior,
        wait_tier: action_primitives::WaitTier,
    },

    /// Wait for condition
    Wait {
        condition: action_primitives::WaitCondition,
        timeout_ms: u64,
    },

    /// Custom action (extensibility point)
    Custom {
        action_type: String,
        parameters: HashMap<String, serde_json::Value>,
    },
}

/// Failure strategy - how to handle step failures
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureStrategy {
    /// Abort entire flow on failure
    Abort,

    /// Continue with next step
    Continue,

    /// Retry step with exponential backoff
    Retry { max_attempts: u32, backoff_ms: u64 },

    /// Use fallback node
    Fallback,
}

/// Flow execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowResult {
    /// Flow identifier
    pub flow_id: String,

    /// Overall success
    pub success: bool,

    /// Start time
    pub started_at: DateTime<Utc>,

    /// Finish time
    pub finished_at: DateTime<Utc>,

    /// Total latency in milliseconds
    pub latency_ms: u64,

    /// Step results
    pub step_results: Vec<StepResult>,

    /// Variables collected during execution
    pub variables: HashMap<String, serde_json::Value>,

    /// Error message if failed
    pub error: Option<String>,
}

impl FlowResult {
    /// Create a new flow result
    pub fn new(flow_id: String) -> Self {
        let now = Utc::now();
        Self {
            flow_id,
            success: false,
            started_at: now,
            finished_at: now,
            latency_ms: 0,
            step_results: Vec::new(),
            variables: HashMap::new(),
            error: None,
        }
    }

    /// Mark as success
    pub fn with_success(mut self) -> Self {
        self.success = true;
        self
    }

    /// Mark as failure
    pub fn with_error(mut self, error: String) -> Self {
        self.success = false;
        self.error = Some(error);
        self
    }

    /// Add step result
    pub fn with_step(mut self, result: StepResult) -> Self {
        self.step_results.push(result);
        self
    }

    /// Set finish time and calculate latency
    pub fn finish(mut self) -> Self {
        self.finished_at = Utc::now();
        self.latency_ms = (self.finished_at - self.started_at).num_milliseconds() as u64;
        self
    }
}

/// Step execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// Step identifier
    pub step_id: String,

    /// Step type
    pub step_type: String,

    /// Success flag
    pub success: bool,

    /// Action report (if applicable)
    pub action_report: Option<ActionReport>,

    /// Start time
    pub started_at: DateTime<Utc>,

    /// Finish time
    pub finished_at: DateTime<Utc>,

    /// Latency in milliseconds
    pub latency_ms: u64,

    /// Retry attempts (if any)
    pub retry_attempts: u32,

    /// Error message (if failed)
    pub error: Option<String>,
}

impl StepResult {
    /// Create a new step result
    pub fn new(step_id: String, step_type: String) -> Self {
        let now = Utc::now();
        Self {
            step_id,
            step_type,
            success: false,
            action_report: None,
            started_at: now,
            finished_at: now,
            latency_ms: 0,
            retry_attempts: 0,
            error: None,
        }
    }

    /// Mark as success
    pub fn with_success(mut self) -> Self {
        self.success = true;
        self
    }

    /// Mark as failure
    pub fn with_error(mut self, error: String) -> Self {
        self.success = false;
        self.error = Some(error);
        self
    }

    /// Add action report
    pub fn with_report(mut self, report: ActionReport) -> Self {
        self.action_report = Some(report);
        self
    }

    /// Set finish time and calculate latency
    pub fn finish(mut self) -> Self {
        self.finished_at = Utc::now();
        self.latency_ms = (self.finished_at - self.started_at).num_milliseconds() as u64;
        self
    }
}

/// Flow execution context
#[derive(Debug, Clone)]
pub struct FlowContext {
    /// Variables accumulated during execution
    pub variables: HashMap<String, serde_json::Value>,

    /// Previous step result
    pub previous_step_success: bool,

    /// Current iteration count (for loops)
    pub iteration_count: u32,
}

impl FlowContext {
    /// Create a new flow context
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            previous_step_success: true,
            iteration_count: 0,
        }
    }

    /// Set variable
    pub fn set_variable(&mut self, name: String, value: serde_json::Value) {
        self.variables.insert(name, value);
    }

    /// Get variable
    pub fn get_variable(&self, name: &str) -> Option<&serde_json::Value> {
        self.variables.get(name)
    }
}

impl Default for FlowContext {
    fn default() -> Self {
        Self::new()
    }
}
