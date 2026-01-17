//! Agent Loop (Observe-Think-Act) execution mode.
//!
//! This module provides browser-use style iterative agent execution where
//! the LLM is consulted at each step to decide the next action based on
//! current browser state, rather than generating a complete plan upfront.
//!
//! # Architecture
//!
//! ```text
//! while !done && steps < max:
//!     state = observe()      // Get current browser state
//!     action = llm.decide()  // LLM decides based on state
//!     result = execute()     // Execute 1-3 actions
//!     if action.is_done: break
//! ```
//!
//! # Key Components
//!
//! - [`AgentLoopConfig`]: Configuration for the agent loop
//! - [`BrowserStateSummary`]: Formatted browser state for LLM consumption
//! - [`AgentOutput`]: LLM output with thinking, memory, and actions
//! - [`AgentLoopController`]: Main loop orchestrator

pub mod config;
pub mod controller;
pub mod element_tree;
pub mod prompt;
pub mod state_formatter;
pub mod types;

pub use config::AgentLoopConfig;
pub use controller::{AgentLoopController, AgentLoopResult, AgentLoopStatus};
pub use element_tree::{
    ElementSelector, ElementSelectorRef, ElementTreeBuilder, ElementTreeResult,
};
pub use prompt::{format_state_update, format_system_prompt, format_user_message};
pub use state_formatter::{PerceptionData, StateFormatter};
pub use types::{
    AgentAction, AgentActionParams, AgentActionType, AgentHistoryEntry, AgentOutput,
    BrowserStateSummary, ScrollDirection, ScrollPosition,
};
