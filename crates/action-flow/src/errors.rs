//! Flow execution error types

use thiserror::Error;

/// Flow execution errors
#[derive(Debug, Error)]
pub enum FlowError {
    /// Flow validation failed
    #[error("Flow validation failed: {0}")]
    ValidationFailed(String),

    /// Step execution failed
    #[error("Step {step_id} failed: {reason}")]
    StepFailed { step_id: String, reason: String },

    /// Condition evaluation failed
    #[error("Condition evaluation failed: {0}")]
    ConditionFailed(String),

    /// Loop exceeded maximum iterations
    #[error("Loop exceeded maximum iterations: {0}")]
    LoopExceeded(u32),

    /// Flow timeout
    #[error("Flow execution timed out after {0}ms")]
    Timeout(u64),

    /// Failure strategy exhausted
    #[error("Failure strategy exhausted for step {0}")]
    StrategyExhausted(String),

    /// Parallel execution error
    #[error("Parallel execution failed: {0}")]
    ParallelFailed(String),

    /// Invalid flow structure
    #[error("Invalid flow structure: {0}")]
    InvalidStructure(String),

    /// Action primitive error
    #[error("Action primitive error: {0}")]
    ActionError(String),

    /// Gate validation error
    #[error("Gate validation error: {0}")]
    GateError(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<action_primitives::ActionError> for FlowError {
    fn from(err: action_primitives::ActionError) -> Self {
        FlowError::ActionError(err.to_string())
    }
}

impl From<action_gate::GateError> for FlowError {
    fn from(err: action_gate::GateError) -> Self {
        FlowError::GateError(err.to_string())
    }
}
