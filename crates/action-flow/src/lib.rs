//! Flow Orchestration Layer
//!
//! This module provides flow orchestration capabilities for browser automation,
//! enabling complex multi-step workflows with conditional logic, loops, and
//! parallel execution.

pub mod errors;
pub mod executor;
pub mod strategies;
pub mod types;

pub use errors::FlowError;
pub use executor::{DefaultFlowExecutor, FlowExecutor};
pub use strategies::{DefaultFailureHandler, FailureHandler};
pub use types::{FailureStrategy, Flow, FlowContext, FlowNode, FlowResult};
