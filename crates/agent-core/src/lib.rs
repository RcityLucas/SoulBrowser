//! L8 Agent Core primitives.
//!
//! Provides shared data models, errors, and conversion utilities for turning
//! conversational agent outputs into executable SoulBrowser flows.

pub mod convert;
pub mod errors;
pub mod llm_provider;
pub mod model;
pub mod plan;
pub mod plan_validator;
pub mod planner;
pub mod weather;

pub use convert::{plan_to_flow, PlanToFlowOptions, PlanToFlowResult};
pub use errors::AgentError;
pub use llm_provider::{LlmProvider, MockLlmProvider};
pub use model::{
    AgentContext, AgentIntentKind, AgentIntentMetadata, AgentRequest, ConversationRole,
    ConversationTurn, RequestedOutput,
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
