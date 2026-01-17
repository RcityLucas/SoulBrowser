//! L8 Agent Core primitives.
//!
//! Provides shared data models, errors, and conversion utilities for turning
//! conversational agent outputs into executable SoulBrowser flows.
//!
//! # Execution Modes
//!
//! This crate supports two execution modes:
//!
//! 1. **Plan-Execute Mode** (default): The LLM generates a complete plan upfront,
//!    which is then executed sequentially. Replanning occurs only on failure.
//!
//! 2. **Agent Loop Mode**: Browser-use style iterative execution where the LLM
//!    is consulted at each step to decide the next action based on current
//!    browser state. See the [`agent_loop`] module for details.

pub mod agent_loop;
pub mod convert;
pub mod errors;
pub mod guardrails;
pub mod llm_provider;
pub mod model;
pub mod plan;
pub mod plan_validator;
pub mod planner;
pub mod weather;

pub use agent_loop::{
    format_state_update, format_system_prompt, format_user_message, AgentAction, AgentActionParams,
    AgentActionType, AgentHistoryEntry, AgentLoopConfig, AgentLoopController, AgentLoopResult,
    AgentLoopStatus, AgentOutput, BrowserStateSummary, ElementSelector, ElementSelectorRef,
    ElementTreeBuilder, ElementTreeResult, PerceptionData, ScrollDirection, ScrollPosition,
    StateFormatter,
};
pub use convert::{plan_to_flow, PlanToFlowOptions, PlanToFlowResult};
pub use errors::AgentError;
pub use guardrails::{derive_guardrail_domains, derive_guardrail_keywords};
pub use llm_provider::{LlmProvider, MockLlmProvider};
pub use model::{
    AgentContext, AgentIntentKind, AgentIntentMetadata, AgentRequest, ConversationRole,
    ConversationTurn, ExecutionMode, RequestedOutput,
};
pub use plan::{
    AgentLocator, AgentPlan, AgentPlanMeta, AgentPlanStep, AgentScrollTarget, AgentTool,
    AgentToolKind, AgentValidation, AgentWaitCondition, WaitMode,
};
pub use plan_validator::{
    is_allowed_custom_tool, requires_user_facing_result, requires_weather_pipeline,
    PlanValidationIssue, PlanValidator,
};
pub use planner::{AgentPlanner, PlannerConfig, PlannerOutcome, RuleBasedPlanner};
pub use weather::{weather_query_text, weather_search_url};
