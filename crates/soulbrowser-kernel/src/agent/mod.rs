pub mod agent_loop_executor;
pub mod executor;
mod guardrails;
pub mod message_manager;
mod stage_context;
pub mod strategies;

pub use agent_loop_executor::{
    execute_agent_loop, AgentLoopExecutionOptions, AgentLoopExecutionReport, AgentLoopStepReport,
};
pub use stage_context::{AutoActTuning, ContextResolver, StageContext};

use agent_core::planner::{classify_step, plan_contains_stage, PlanStageGraph, PlanStageKind};
use agent_core::{
    is_allowed_custom_tool, plan_to_flow, requires_user_facing_result, requires_weather_pipeline,
    weather_query_text, AgentContext, AgentError, AgentIntentKind, AgentLocator, AgentPlan,
    AgentPlanStep, AgentPlanner, AgentRequest, AgentScrollTarget, AgentTool, AgentToolKind,
    AgentValidation, AgentWaitCondition, ConversationRole, ConversationTurn, LlmProvider,
    PlanToFlowOptions, PlanToFlowResult, PlanValidationIssue, PlanValidator, PlannerConfig,
    PlannerOutcome, RuleBasedPlanner, WaitMode,
};
use anyhow::{anyhow, Context, Result};
use hex;
use once_cell::sync::OnceCell;
use regex::escape;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use soulbrowser_core_types::TaskId;
use std::{collections::HashMap, fmt, sync::Arc};
use tracing::{debug, warn};
use url::{form_urlencoded, Url};

use crate::agent::guardrails::{derive_guardrail_domains, derive_guardrail_keywords};
use crate::agent::strategies::{
    materialize_step, stage_label, stage_overlay, StrategyInput, StrategyRegistry,
};
use crate::llm::LlmPlanCache;
use crate::metrics::{
    record_auto_repair_events, record_plan_rejection, record_strategy_usage, record_template_usage,
};

pub use executor::{execute_plan, FlowExecutionOptions, FlowExecutionReport, StepExecutionStatus};

/// Runner that bridges CLI prompts to either the rule-based or LLM planner.
#[derive(Clone)]
pub struct ChatRunner {
    planner: PlannerStrategy,
    flow_options: PlanToFlowOptions,
    strict_plan_validation: bool,
}

impl fmt::Debug for ChatRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatRunner")
            .field("planner", &self.planner)
            .field("strict_plan_validation", &self.strict_plan_validation)
            .finish()
    }
}

#[derive(Clone)]
enum PlannerStrategy {
    Rule(RuleBasedPlanner),
    Llm {
        planner: LlmPlanner,
        fallback: RuleBasedPlanner,
    },
}

impl fmt::Debug for PlannerStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlannerStrategy::Rule(_) => f.write_str("PlannerStrategy::Rule"),
            PlannerStrategy::Llm { planner, .. } => f
                .debug_struct("PlannerStrategy::Llm")
                .field("cache", &planner.cache.is_some())
                .finish(),
        }
    }
}

impl PlannerStrategy {
    async fn plan(&self, request: &AgentRequest) -> Result<PlannerOutcome, AgentError> {
        match self {
            PlannerStrategy::Rule(planner) => {
                let mut outcome = planner.draft_plan(request)?;
                annotate_plan_origin(&mut outcome.plan, "rule");
                Ok(outcome)
            }
            PlannerStrategy::Llm { planner, fallback } => match planner.plan(request).await {
                Ok(mut outcome) => {
                    annotate_plan_origin(&mut outcome.plan, "llm");
                    Ok(outcome)
                }
                Err(err) => {
                    warn!("LLM planner failed; falling back to rule plan: {}", err);
                    let mut outcome = fallback.draft_plan(request)?;
                    annotate_plan_origin(&mut outcome.plan, "rule_fallback");
                    Ok(outcome)
                }
            },
        }
    }

    async fn replan(
        &self,
        request: &AgentRequest,
        previous_plan: &AgentPlan,
        failure_summary: &str,
    ) -> Result<PlannerOutcome, AgentError> {
        match self {
            PlannerStrategy::Rule(planner) => {
                let mut outcome = planner.draft_plan(request)?;
                annotate_plan_origin(&mut outcome.plan, "rule");
                Ok(outcome)
            }
            PlannerStrategy::Llm { planner, fallback } => {
                match planner
                    .replan(request, previous_plan, failure_summary)
                    .await
                {
                    Ok(mut outcome) => {
                        annotate_plan_origin(&mut outcome.plan, "llm");
                        Ok(outcome)
                    }
                    Err(err) => {
                        warn!("LLM replanner failed; falling back to rule plan: {}", err);
                        let mut outcome = fallback.draft_plan(request)?;
                        annotate_plan_origin(&mut outcome.plan, "rule_fallback");
                        Ok(outcome)
                    }
                }
            }
        }
    }
}

fn annotate_plan_origin(plan: &mut AgentPlan, kind: &str) {
    plan.meta
        .vendor_context
        .insert("planner_kind".to_string(), Value::String(kind.to_string()));
}

struct LlmPlanner {
    provider: Arc<dyn LlmProvider>,
    cache: Option<Arc<LlmPlanCache>>,
}

impl Clone for LlmPlanner {
    fn clone(&self) -> Self {
        Self {
            provider: Arc::clone(&self.provider),
            cache: self.cache.as_ref().map(Arc::clone),
        }
    }
}

impl fmt::Debug for LlmPlanner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlmPlanner")
            .field("cache_enabled", &self.cache.is_some())
            .finish()
    }
}

impl LlmPlanner {
    fn new(provider: Arc<dyn LlmProvider>, cache: Option<Arc<LlmPlanCache>>) -> Self {
        Self { provider, cache }
    }

    async fn plan(&self, request: &AgentRequest) -> Result<PlannerOutcome, AgentError> {
        if let Some(cache) = &self.cache {
            if let Some(key) = cache_key_for_request(request) {
                if let Some(entry) = cache.load_plan(&key).await {
                    return Ok(PlannerOutcome {
                        plan: entry.plan,
                        explanations: entry.explanations,
                    });
                }
            }
        }

        let outcome = self.provider.plan(request).await?;
        if let Some(cache) = &self.cache {
            if let Some(key) = cache_key_for_request(request) {
                cache
                    .store_plan(&key, &outcome.plan, &outcome.explanations)
                    .await;
            }
        }
        Ok(outcome)
    }

    async fn replan(
        &self,
        request: &AgentRequest,
        previous_plan: &AgentPlan,
        failure_summary: &str,
    ) -> Result<PlannerOutcome, AgentError> {
        self.provider
            .replan(request, previous_plan, failure_summary)
            .await
    }
}

impl Default for ChatRunner {
    fn default() -> Self {
        Self::with_config(PlannerConfig::default(), PlanToFlowOptions::default())
    }
}

impl ChatRunner {
    pub fn with_config(config: PlannerConfig, flow_options: PlanToFlowOptions) -> Self {
        let strict_plan_validation = config.strict_plan_validation;
        let rule = RuleBasedPlanner::new(config);
        Self {
            planner: PlannerStrategy::Rule(rule),
            flow_options,
            strict_plan_validation,
        }
    }

    pub fn with_llm_provider(self, provider: Arc<dyn LlmProvider>) -> Self {
        self.with_llm_backend(provider, None)
    }

    pub fn with_llm_backend(
        mut self,
        provider: Arc<dyn LlmProvider>,
        cache: Option<Arc<LlmPlanCache>>,
    ) -> Self {
        let fallback = match &self.planner {
            PlannerStrategy::Rule(rule) => rule.clone(),
            PlannerStrategy::Llm { fallback, .. } => fallback.clone(),
        };
        self.planner = PlannerStrategy::Llm {
            planner: LlmPlanner::new(provider, cache),
            fallback,
        };
        self
    }

    /// Build an `AgentRequest` from a plain prompt, optional context, and constraints.
    pub fn request_from_prompt(
        &self,
        prompt: String,
        context: Option<AgentContext>,
        constraints: Vec<String>,
    ) -> AgentRequest {
        let mut request = AgentRequest::new(TaskId::new(), prompt.clone());
        request.push_turn(ConversationTurn::new(ConversationRole::User, prompt));
        request.constraints = constraints;
        if let Some(ctx) = context {
            request = request.with_context(ctx);
        }
        request
    }

    /// Generate a plan and flow given the prepared request envelope.
    pub async fn plan(&self, mut request: AgentRequest) -> Result<ChatSessionOutput> {
        ensure_prompt(&request)?;
        ensure_conversation(&mut request);

        let outcome = self
            .planner
            .plan(&request)
            .await
            .map_err(|err| anyhow!("planner failed: {}", err))?;
        self.finalize_with_schema_retry(&request, outcome).await
    }

    /// Re-plan after a failed execution attempt.
    pub async fn replan(
        &self,
        mut request: AgentRequest,
        previous_plan: &AgentPlan,
        failure_summary: &str,
    ) -> Result<ChatSessionOutput> {
        ensure_prompt(&request)?;
        ensure_conversation(&mut request);

        let outcome = self
            .planner
            .replan(&request, previous_plan, failure_summary)
            .await
            .map_err(|err| anyhow!("planner failed: {}", err))?;
        let mut session = self.finalize_with_schema_retry(&request, outcome).await?;
        append_plan_repair_note(&mut session.plan, failure_summary);
        Ok(session)
    }

    async fn finalize_with_schema_retry(
        &self,
        request: &AgentRequest,
        outcome: PlannerOutcome,
    ) -> Result<ChatSessionOutput> {
        let previous_plan_snapshot = outcome.plan.clone();
        match self.finalize_outcome(outcome, request) {
            Ok(output) => Ok(output),
            Err(err) => {
                if let Some(issue) = err.downcast_ref::<PlanValidationIssue>() {
                    record_plan_rejection(issue.telemetry_label());
                    if issue.should_trigger_replan() {
                        let failure_summary = format!("Plan validation failed: {}", issue);
                        let replanned = self
                            .planner
                            .replan(request, &previous_plan_snapshot, &failure_summary)
                            .await
                            .map_err(|planner_err| anyhow!("planner failed: {}", planner_err))?;
                        return self.finalize_outcome(replanned, request);
                    }
                }
                Err(err)
            }
        }
    }

    fn finalize_outcome(
        &self,
        mut outcome: PlannerOutcome,
        request: &AgentRequest,
    ) -> Result<ChatSessionOutput> {
        let repair_report = normalize_plan(&mut outcome.plan, request);
        if repair_report.has_repairs() {
            debug!(
                count = repair_report.total_repairs,
                "auto-repaired planner output"
            );
            attach_repair_metadata(&mut outcome.plan, &repair_report);
            if let Some(summary) = repair_summary(&repair_report) {
                outcome.explanations.push(summary.clone());
                if matches!(self.planner, PlannerStrategy::Llm { .. }) {
                    outcome
                        .plan
                        .meta
                        .vendor_context
                        .insert("planner_critiques".to_string(), json!([summary]));
                }
            }
        }
        if let Some(Value::String(recipe)) = outcome.plan.meta.vendor_context.get("intent_recipe") {
            record_template_usage(recipe);
        }
        apply_execution_tweaks(&mut outcome.plan);
        let validator = if self.strict_plan_validation {
            PlanValidator::strict()
        } else {
            PlanValidator::default()
        };
        validator
            .validate(&outcome.plan, request)
            .map_err(|err| anyhow!(err))
            .context("plan failed validation")?;
        let flow = plan_to_flow(&outcome.plan, self.flow_options.clone())
            .map_err(|err| anyhow!("plan conversion failed: {}", err))?;

        Ok(ChatSessionOutput {
            plan: outcome.plan,
            explanations: outcome.explanations,
            flow,
        })
    }
}

fn ensure_prompt(request: &AgentRequest) -> Result<()> {
    if request.goal.trim().is_empty() {
        Err(anyhow!("Prompt cannot be empty"))
    } else {
        Ok(())
    }
}

fn ensure_conversation(request: &mut AgentRequest) {
    if request.conversation.is_empty() {
        request.push_turn(ConversationTurn::new(
            ConversationRole::User,
            request.goal.clone(),
        ));
    }
}

fn cache_key_for_request(request: &AgentRequest) -> Option<String> {
    let mut metadata_entries: Vec<_> = request.metadata.iter().collect();
    metadata_entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut metadata = Map::<String, Value>::with_capacity(metadata_entries.len());
    for (key, value) in metadata_entries {
        metadata.insert(key.clone(), value.clone());
    }

    let canonical = json!({
        "goal": request.goal.trim(),
        "constraints": request.constraints,
        "current_url": request
            .context
            .as_ref()
            .and_then(|ctx| ctx.current_url.as_deref())
            .unwrap_or_default(),
        "metadata": metadata,
    });
    let bytes = serde_json::to_vec(&canonical).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Some(hex::encode(hasher.finalize()))
}

const OBSERVATION_CANONICAL: &str = "data.extract-site";
const GENERIC_PARSE_CANONICAL: &str = "data.parse.generic";
const DELIVER_CANONICAL: &str = "data.deliver.structured";
pub(super) const PLUGIN_CUSTOM_ALIAS_CASES: &[(&str, &str)] = &[
    ("plugin.extract-site", OBSERVATION_CANONICAL),
    ("plugin.data-parse-metal-price", "data.parse.metal_price"),
    ("plugin.data-deliver-structured", DELIVER_CANONICAL),
    ("plugin.data-validate-target", "data.validate-target"),
    (
        "plugin.data-validate-metal-price",
        "data.validate.metal_price",
    ),
    ("plugin.data-parse.generic", GENERIC_PARSE_CANONICAL),
    ("plugin.browser.search", "browser.search"),
    ("plugin.close-modal", "browser.close-modal"),
    ("plugin.send-esc", "browser.send-esc"),
];
pub(crate) const EXPECTED_URL_METADATA_KEY: &str = "expected_url";
pub(crate) const SKIP_CLICK_VALIDATION_METADATA_KEY: &str = "skip_click_validation";
static PLAN_STAGE_GRAPH: OnceCell<PlanStageGraph> = OnceCell::new();

#[cfg(test)]
fn normalize_custom_tools(plan: &mut AgentPlan) -> usize {
    normalize_custom_tools_with_handler(plan, |_, _| {})
}

fn stage_graph() -> &'static PlanStageGraph {
    PLAN_STAGE_GRAPH.get_or_init(|| PlanStageGraph::load_from_env_or_default().unwrap_or_default())
}

fn normalize_custom_tools_with_handler<F>(plan: &mut AgentPlan, mut on_change: F) -> usize
where
    F: FnMut(&mut AgentPlanStep, &str),
{
    let mut rewrites = 0;
    for step in plan.steps.iter_mut() {
        let previous_name = match &step.tool.kind {
            AgentToolKind::Custom { name, .. } => Some(name.clone()),
            _ => None,
        };
        if normalize_step_tool(step) {
            let note = match (&previous_name, &step.tool.kind) {
                (Some(prev), AgentToolKind::Custom { name, .. }) => {
                    format!("Normalized custom tool '{}' -> '{}'", prev, name)
                }
                (Some(prev), _) => format!("Rewrote tool alias '{}' into builtin action", prev),
                _ => "Normalized tool alias".to_string(),
            };
            on_change(step, &note);
            rewrites += 1;
        }
    }
    rewrites
}

const PLAN_REPAIR_NOTE_BUDGET: usize = 12;

#[derive(Debug, Default, Clone)]
struct PlanRepairReport {
    total_repairs: usize,
    notes: Vec<String>,
    budget_exhausted: bool,
    overlays: Vec<Value>,
}

impl PlanRepairReport {
    fn has_repairs(&self) -> bool {
        self.total_repairs > 0
    }
}

struct PlanRepairLedger {
    total_repairs: usize,
    notes: Vec<String>,
    note_budget: usize,
    budget_exhausted: bool,
    overlays: Vec<Value>,
}

impl PlanRepairLedger {
    fn new(note_budget: usize) -> Self {
        Self {
            total_repairs: 0,
            notes: Vec::new(),
            note_budget,
            budget_exhausted: false,
            overlays: Vec::new(),
        }
    }

    fn mark_step(&mut self, step: &mut AgentPlanStep, note: impl Into<String>) {
        let note = note.into();
        mark_step_repaired(step, &note);
        self.push_note(note);
    }

    fn record_note(&mut self, note: impl Into<String>) {
        self.push_note(note.into());
    }

    fn record_overlay(&mut self, overlay: Value) {
        self.overlays.push(overlay);
    }

    fn push_note(&mut self, note: String) {
        self.total_repairs += 1;
        if self.notes.len() < self.note_budget {
            self.notes.push(note);
        } else if !self.budget_exhausted {
            warn!(
                limit = self.note_budget,
                "plan repair note budget exhausted; suppressing additional notes"
            );
            self.budget_exhausted = true;
        }
    }

    fn into_report(self) -> PlanRepairReport {
        PlanRepairReport {
            total_repairs: self.total_repairs,
            notes: self.notes,
            budget_exhausted: self.budget_exhausted,
            overlays: self.overlays,
        }
    }
}

struct StageAuditor<'a> {
    plan: &'a mut AgentPlan,
    request: &'a AgentRequest,
    context: StageContext,
    ledger: &'a mut PlanRepairLedger,
    registry: StrategyRegistry,
    force_deterministic: bool,
    stage_timeline: Vec<Value>,
}

enum StageOutcome {
    AlreadyPresent,
    StrategyApplied(String),
    PlaceholderInserted,
    Missing,
}

impl<'a> StageAuditor<'a> {
    fn new(
        plan: &'a mut AgentPlan,
        request: &'a AgentRequest,
        context: StageContext,
        ledger: &'a mut PlanRepairLedger,
        force_deterministic: bool,
    ) -> Self {
        Self {
            plan,
            request,
            context,
            ledger,
            registry: StrategyRegistry::builtin(),
            force_deterministic,
            stage_timeline: Vec::new(),
        }
    }

    fn audit(&mut self) {
        self.record_guardrail_overlay();
        let stage_plan = stage_graph().plan_for_request(self.request);
        if self.force_deterministic {
            self.reset_plan_for_deterministic();
        } else {
            self.retarget_blocked_search_engines();
            self.align_search_observations();
        }
        for chain in stage_plan.stages.iter() {
            let outcome = if self.stage_already_satisfied(chain.stage) {
                StageOutcome::AlreadyPresent
            } else {
                self.try_chain(chain)
            };
            self.emit_stage_status(chain.stage, outcome);
        }
        if !self.force_deterministic {
            self.retarget_blocked_search_engines();
        }
        self.persist_stage_timeline();
    }

    fn reset_plan_for_deterministic(&mut self) {
        if self.plan.steps.is_empty() {
            return;
        }
        self.plan.steps.clear();
        self.ledger
            .record_note("LLM plan overridden by deterministic informational pipeline");
        let mut overlay = stage_overlay(
            PlanStageKind::Navigate,
            "deterministic_plan",
            "reset",
            "♻️ 使用固定阶段图重建计划",
        );
        if let Some(obj) = overlay.as_object_mut() {
            obj.insert(
                "reason".to_string(),
                Value::String("informational_intent".to_string()),
            );
        }
        self.ledger.record_overlay(overlay);
    }

    fn record_guardrail_overlay(&mut self) {
        if self.context.guardrail_keywords.is_empty() {
            return;
        }
        let keywords = self.context.guardrail_keywords.clone();
        let count = keywords.len();
        let preview = keywords
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(" / ");
        let detail = format!("注入 {} 个 Guardrail 关键词：{}", count, preview);
        let overlay = json!({
            "kind": "guardrail_keywords",
            "title": "🎯 Guardrail 关键词注入",
            "detail": detail,
            "message": detail,
            "badge": {
                "label": "Guardrail",
                "value": count,
                "tone": "info",
            },
            "keywords": keywords.clone(),
            "domains": self.context.guardrail_domains.clone(),
        });
        self.ledger.record_overlay(overlay);
        self.plan.meta.vendor_context.insert(
            "guardrail_keywords".to_string(),
            json!({
                "keywords": keywords,
                "count": count,
                "domains": self.context.guardrail_domains.clone(),
                "emitted": false,
            }),
        );
    }

    fn stage_already_satisfied(&self, stage: PlanStageKind) -> bool {
        match stage {
            PlanStageKind::Navigate => {
                if self.should_prioritize_search_navigation() && !plan_has_browser_search(self.plan)
                {
                    return false;
                }
                plan_has_navigate_step(self.plan)
            }
            PlanStageKind::Act => plan_has_auto_act(self.plan),
            PlanStageKind::Observe => plan_has_extract_site(self.plan),
            PlanStageKind::Validate => plan_has_target_validation(self.plan),
            PlanStageKind::Parse => plan_has_parse_step(self.plan),
            PlanStageKind::Deliver => plan_has_deliver_stage(self.plan),
            _ => plan_contains_stage(self.plan, stage),
        }
    }

    fn try_chain(&mut self, chain: &agent_core::planner::StageStrategyChain) -> StageOutcome {
        if chain.stage == PlanStageKind::Navigate && self.should_prioritize_search_navigation() {
            if let Some(outcome) = self.try_specific_strategy(chain.stage, "search") {
                return outcome;
            }
        }

        for strategy_id in &chain.strategies {
            let Some(strategy) = self.registry.get(strategy_id) else {
                continue;
            };
            let application = {
                let input = StrategyInput {
                    plan: self.plan,
                    request: self.request,
                    context: &self.context,
                };
                strategy.apply(&input)
            };
            if let Some(result) = application {
                record_strategy_usage(chain.stage.as_str(), strategy_id, "applied");
                self.apply_result(chain.stage, strategy.id(), result);
                return StageOutcome::StrategyApplied(strategy.id().to_string());
            } else {
                record_strategy_usage(chain.stage.as_str(), strategy_id, "skipped");
            }
        }
        record_strategy_usage(chain.stage.as_str(), "none", "exhausted");
        if self.synthesize_placeholder(chain.stage) {
            record_strategy_usage(chain.stage.as_str(), "placeholder", "applied");
            StageOutcome::PlaceholderInserted
        } else {
            record_strategy_usage(chain.stage.as_str(), "placeholder", "skipped");
            StageOutcome::Missing
        }
    }

    fn try_specific_strategy(
        &mut self,
        stage: PlanStageKind,
        strategy_id: &str,
    ) -> Option<StageOutcome> {
        let Some(strategy) = self.registry.get(strategy_id) else {
            return None;
        };
        let input = StrategyInput {
            plan: self.plan,
            request: self.request,
            context: &self.context,
        };
        let application = strategy.apply(&input);
        match application {
            Some(result) => {
                record_strategy_usage(stage.as_str(), strategy_id, "applied");
                self.apply_result(stage, strategy.id(), result);
                Some(StageOutcome::StrategyApplied(strategy.id().to_string()))
            }
            None => {
                record_strategy_usage(stage.as_str(), strategy_id, "skipped");
                None
            }
        }
    }

    fn should_prioritize_search_navigation(&self) -> bool {
        if requires_weather_pipeline(self.request) {
            return false;
        }
        if self.context.guardrail_keyword_count > 0 {
            return true;
        }
        if self.context.preferred_sites.is_empty() && !self.context.search_terms.is_empty() {
            return true;
        }
        false
    }

    fn emit_stage_status(&mut self, stage: PlanStageKind, outcome: StageOutcome) {
        let label = stage_label(stage);
        let (strategy, status, detail) = match outcome {
            StageOutcome::AlreadyPresent => (
                "plan".to_string(),
                "existing".to_string(),
                format!("✅ 计划已覆盖{}阶段", label),
            ),
            StageOutcome::StrategyApplied(id) => (
                id.clone(),
                "auto_strategy".to_string(),
                format!("🧠 策略 {} 补齐{}阶段", id, label),
            ),
            StageOutcome::PlaceholderInserted => (
                "placeholder".to_string(),
                "placeholder".to_string(),
                format!("⚙️ 使用占位步骤补齐{}阶段", label),
            ),
            StageOutcome::Missing => (
                "missing".to_string(),
                "missing".to_string(),
                format!("⚠️ 仍缺少{}阶段，请检查任务提示", label),
            ),
        };
        self.ledger.record_overlay(stage_overlay(
            stage,
            strategy.clone(),
            status.clone(),
            detail.clone(),
        ));
        self.stage_timeline.push(json!({
            "stage": stage.as_str(),
            "label": label,
            "status": status,
            "strategy": strategy,
            "detail": detail,
        }));
    }

    fn apply_result(
        &mut self,
        stage: PlanStageKind,
        strategy_id: &str,
        application: strategies::StrategyApplication,
    ) {
        if application.steps.is_empty() {
            return;
        }
        let mut insert_at = insertion_index(self.plan, stage);
        for template in application.steps.iter() {
            let base_id = format!("stage-{}", stage.as_str());
            let step_id = unique_step_id(self.plan, &base_id);
            let mut step = materialize_step(template, step_id);
            let note = format!(
                "Stage '{}' satisfied via strategy '{}'.",
                stage.as_str(),
                strategy_id
            );
            self.ledger.mark_step(&mut step, note);
            self.plan.steps.insert(insert_at, step);
            insert_at += 1;
        }
        record_auto_repair_events("stage_strategy", application.steps.len() as u64);
        if let Some(note) = application.note {
            self.ledger.record_note(note);
        }
        if let Some(overlay) = application.overlay {
            self.ledger.record_overlay(overlay);
        }
        if !application.vendor_context.is_empty() {
            for (key, value) in application.vendor_context {
                self.plan.meta.vendor_context.insert(key, value);
            }
        }
    }

    fn synthesize_placeholder(&mut self, stage: PlanStageKind) -> bool {
        match stage {
            PlanStageKind::Navigate => self.insert_placeholder_navigate(),
            PlanStageKind::Observe => self.insert_placeholder_observe(),
            PlanStageKind::Validate => self.insert_placeholder_validate(),
            PlanStageKind::Act => self.insert_placeholder_act(),
            PlanStageKind::Evaluate => self.insert_placeholder_evaluate(),
            PlanStageKind::Parse => self.insert_placeholder_parse(),
            PlanStageKind::Deliver => self.insert_placeholder_deliver(),
        }
    }

    fn insert_placeholder_act(&mut self) -> bool {
        let mut step = AgentPlanStep {
            id: unique_step_id(self.plan, "placeholder-act"),
            title: "探索页面可交互元素".to_string(),
            detail: "Fallback act stage via scroll".to_string(),
            tool: AgentTool {
                kind: AgentToolKind::Scroll {
                    target: AgentScrollTarget::Pixels(640),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(4_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        };
        self.ledger
            .mark_step(&mut step, "Placeholder act step inserted");
        self.ledger.record_overlay(stage_overlay(
            PlanStageKind::Act,
            "placeholder",
            "placeholder_step",
            "🕹️ 自动补齐执行阶段",
        ));
        let insert_index = insertion_index(self.plan, PlanStageKind::Act);
        self.plan.steps.insert(insert_index, step);
        true
    }

    fn insert_placeholder_evaluate(&mut self) -> bool {
        let mut step = AgentPlanStep {
            id: unique_step_id(self.plan, "placeholder-evaluate"),
            title: "评估页面状态".to_string(),
            detail: "Fallback evaluate stage via agent.evaluate".to_string(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "agent.evaluate".to_string(),
                    payload: json!({
                        "message": "评估当前页面状态",
                    }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(1_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        };
        let insert_index = insertion_index(self.plan, PlanStageKind::Evaluate);
        self.ledger
            .mark_step(&mut step, "Placeholder agent.evaluate inserted");
        self.plan.steps.insert(insert_index, step);
        self.ledger.record_overlay(stage_overlay(
            PlanStageKind::Evaluate,
            "placeholder",
            "placeholder_step",
            "🧐 自动补齐评估阶段",
        ));
        true
    }

    fn insert_placeholder_navigate(&mut self) -> bool {
        let url = self
            .context
            .best_known_url()
            .unwrap_or_else(|| self.context.fallback_search_url());
        let mut step = AgentPlanStep {
            id: unique_step_id(self.plan, "placeholder-navigate"),
            title: "自动跳转页面".to_string(),
            detail: format!("Fallback navigation to {url}"),
            tool: AgentTool {
                kind: AgentToolKind::Navigate { url: url.clone() },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        };
        self.ledger
            .mark_step(&mut step, format!("Placeholder navigate -> {url}"));
        self.ledger.record_overlay(stage_overlay(
            PlanStageKind::Navigate,
            "placeholder",
            "placeholder_step",
            "⚠️ 自动补齐导航阶段",
        ));
        self.plan.steps.insert(0, step);
        true
    }

    fn insert_placeholder_observe(&mut self) -> bool {
        let url = self
            .plan
            .steps
            .iter()
            .rev()
            .find_map(|step| match &step.tool.kind {
                AgentToolKind::Navigate { url } => Some(url.clone()),
                _ => None,
            })
            .or_else(|| self.context.best_known_url())
            .unwrap_or_else(|| self.context.fallback_search_url());
        let mut step = AgentPlanStep {
            id: unique_step_id(self.plan, "placeholder-observe"),
            title: "自动采集页面".to_string(),
            detail: "Fallback observation".to_string(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.extract-site".to_string(),
                    payload: json!({
                        "title": "自动采集页面内容",
                        "detail": "Placeholder observation",
                        "url": url,
                    }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(10_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        };
        step.metadata.insert(
            EXPECTED_URL_METADATA_KEY.to_string(),
            Value::String(url.clone()),
        );
        let insert_index = insertion_index(self.plan, PlanStageKind::Observe);
        self.ledger
            .mark_step(&mut step, "Placeholder observation inserted");
        self.ledger.record_overlay(stage_overlay(
            PlanStageKind::Observe,
            "placeholder",
            "placeholder_step",
            "📸 自动补齐观察阶段",
        ));
        self.plan.steps.insert(insert_index, step);
        true
    }

    fn insert_placeholder_validate(&mut self) -> bool {
        let Some((_, observation_id)) = previous_observation_step(self.plan, self.plan.steps.len())
        else {
            return false;
        };
        let keywords = derive_guardrail_keywords(self.request);
        let allowed_domains = derive_guardrail_domains(self.request);
        if keywords.is_empty() && allowed_domains.is_empty() {
            return false;
        }
        let mut step = AgentPlanStep {
            id: unique_step_id(self.plan, "placeholder-validate"),
            title: "验证目标页面".to_string(),
            detail: "Placeholder target validation".to_string(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.validate-target".to_string(),
                    payload: json!({
                        "source_step_id": observation_id,
                        "keywords": keywords,
                        "allowed_domains": allowed_domains,
                        "expected_status": 200,
                    }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(3_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        };
        let insert_index = insertion_index(self.plan, PlanStageKind::Validate);
        self.ledger
            .mark_step(&mut step, "Placeholder validation inserted");
        self.ledger.record_overlay(stage_overlay(
            PlanStageKind::Validate,
            "placeholder",
            "placeholder_step",
            "🛡️ 自动补齐校验阶段",
        ));
        self.plan.steps.insert(insert_index, step);
        true
    }

    fn insert_placeholder_parse(&mut self) -> bool {
        if !plan_has_observation_step(self.plan) {
            self.insert_placeholder_observe();
        }
        let Some((_, observation_id)) = previous_observation_step(self.plan, self.plan.steps.len())
        else {
            return false;
        };
        let mut step = AgentPlanStep {
            id: unique_step_id(self.plan, "placeholder-parse"),
            title: "自动解析数据".to_string(),
            detail: "Placeholder parser".to_string(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.parse.generic".to_string(),
                    payload: json!({
                        "source_step_id": observation_id,
                        "schema": "generic_observation_v1",
                        "title": "Auto parser",
                        "detail": "Placeholder parser",
                    }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(5_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        };
        let insert_index = insertion_index(self.plan, PlanStageKind::Parse);
        self.ledger
            .mark_step(&mut step, "Placeholder parse inserted");
        self.ledger.record_overlay(stage_overlay(
            PlanStageKind::Parse,
            "placeholder",
            "placeholder_step",
            "🧠 自动补齐解析阶段",
        ));
        self.plan.steps.insert(insert_index, step);
        self.insert_placeholder_deliver();
        true
    }

    fn insert_placeholder_deliver(&mut self) -> bool {
        if plan_has_deliver_step(self.plan) || plan_has_note_step(self.plan) {
            return true;
        }
        let mut step = build_auto_note_step(self.plan, self.request);
        self.ledger
            .mark_step(&mut step, "Placeholder agent.note inserted");
        self.ledger.record_overlay(stage_overlay(
            PlanStageKind::Deliver,
            "placeholder",
            "placeholder_step",
            "📝 自动补齐输出阶段",
        ));
        self.plan.steps.push(step);
        true
    }

    fn align_search_observations(&mut self) {
        if self.context.search_terms.is_empty() {
            return;
        }
        let target_url = self.context.fallback_search_url();
        for step in self.plan.steps.iter_mut() {
            if !is_observation_step(step) {
                continue;
            }
            let AgentToolKind::Custom { payload, .. } = &mut step.tool.kind else {
                continue;
            };
            if !payload.is_object() {
                *payload = json!({});
            }
            let map = payload
                .as_object_mut()
                .expect("observation payload should be object");
            let current_url = map
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !current_url.contains("baidu.com") || current_url.contains("baidu.com/s?") {
                continue;
            }
            map.insert("url".to_string(), Value::String(target_url.clone()));
            self.ledger
                .mark_step(step, format!("Retarget observation to {target_url}"));
            step.metadata.insert(
                EXPECTED_URL_METADATA_KEY.to_string(),
                Value::String(target_url.clone()),
            );
            ensure_observe_validations(step, &target_url, self.ledger);
            let mut overlay = stage_overlay(
                PlanStageKind::Observe,
                "search_align",
                "adjust",
                "🔄 观察改为搜索结果页",
            );
            if let Some(obj) = overlay.as_object_mut() {
                obj.insert("step_id".to_string(), Value::String(step.id.clone()));
            }
            self.ledger.record_overlay(overlay);
        }
    }

    fn retarget_blocked_search_engines(&mut self) {
        let fallback_url = self.context.fallback_search_url();
        if fallback_url.is_empty() || is_blocked_search_engine(&fallback_url) {
            return;
        }
        let mut rewrote_navigation = false;
        let fallback_condition = build_url_wait_condition(&fallback_url);
        for step in self.plan.steps.iter_mut() {
            match &mut step.tool.kind {
                AgentToolKind::Navigate { url } => {
                    if !is_blocked_search_engine(url) {
                        continue;
                    }
                    let previous = url.clone();
                    *url = fallback_url.clone();
                    if step.detail.trim().is_empty() {
                        step.detail = format!("打开搜索结果：{}", self.context.search_seed());
                    }
                    step.metadata.insert(
                        EXPECTED_URL_METADATA_KEY.to_string(),
                        Value::String(fallback_url.clone()),
                    );
                    self.ledger.mark_step(
                        step,
                        format!(
                            "Search engine '{previous}' replaced with fallback '{fallback}'",
                            fallback = fallback_url
                        ),
                    );
                    rewrote_navigation = true;
                }
                AgentToolKind::Wait { condition } => {
                    if wait_condition_targets_blocked_search(condition) {
                        *condition = fallback_condition.clone();
                        self.ledger
                            .record_note(format!("Wait condition retargeted to {}", fallback_url));
                    }
                }
                AgentToolKind::Custom { name, payload }
                    if name.eq_ignore_ascii_case("wait-for-condition") =>
                {
                    if wait_for_condition_payload_targets_blocked_search(payload) {
                        if retarget_wait_for_condition_payload(payload, &fallback_condition) {
                            self.ledger.record_note(format!(
                                "wait-for-condition retargeted to {}",
                                fallback_url
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        if rewrote_navigation {
            let mut overlay = stage_overlay(
                PlanStageKind::Navigate,
                "search_engine_fallback",
                "adjust",
                "🔍 搜索引擎不可用，改用备用入口",
            );
            if let Some(obj) = overlay.as_object_mut() {
                obj.insert(
                    "seed".to_string(),
                    Value::String(self.context.search_seed()),
                );
            }
            self.ledger.record_overlay(overlay);
        }
    }

    fn persist_stage_timeline(&mut self) {
        if self.stage_timeline.is_empty() {
            return;
        }
        self.plan.meta.vendor_context.insert(
            "stage_timeline".to_string(),
            json!({
                "stages": self.stage_timeline,
                "deterministic": self.force_deterministic,
            }),
        );
    }
}

fn is_blocked_search_engine(url: &str) -> bool {
    let lowered = url.to_ascii_lowercase();
    lowered.contains("google.") || lowered.contains("bing.")
}

fn wait_condition_targets_blocked_search(condition: &AgentWaitCondition) -> bool {
    match condition {
        AgentWaitCondition::UrlMatches(pattern) | AgentWaitCondition::UrlEquals(pattern) => {
            is_blocked_search_engine(pattern)
        }
        _ => false,
    }
}

fn wait_for_condition_payload_targets_blocked_search(payload: &Value) -> bool {
    if let Some(blocked) = payload
        .as_object()
        .and_then(|obj| obj.get("expect"))
        .and_then(Value::as_object)
        .map(|expect| {
            expect
                .get("url_pattern")
                .and_then(Value::as_str)
                .map(is_blocked_search_engine)
                .unwrap_or(false)
                || expect
                    .get("url_equals")
                    .and_then(Value::as_str)
                    .map(is_blocked_search_engine)
                    .unwrap_or(false)
        })
    {
        if blocked {
            return true;
        }
    }

    let serialized = payload.to_string().to_ascii_lowercase();
    serialized.contains("google.") || serialized.contains("bing.")
}

fn retarget_wait_for_condition_payload(
    payload: &mut Value,
    condition: &AgentWaitCondition,
) -> bool {
    if payload.as_object().is_none() {
        *payload = Value::Object(Map::new());
    }
    let map = payload
        .as_object_mut()
        .expect("payload should be object after normalization");
    let expect = map
        .entry("expect".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .expect("expect should be object");
    match condition {
        AgentWaitCondition::UrlMatches(pattern) => {
            expect.insert("url_pattern".to_string(), Value::String(pattern.clone()));
            expect.remove("url_equals");
            true
        }
        AgentWaitCondition::UrlEquals(url) => {
            expect.insert("url_equals".to_string(), Value::String(url.clone()));
            expect.remove("url_pattern");
            true
        }
        _ => false,
    }
}

fn retarget_wait_tools(
    plan: &mut AgentPlan,
    context: &StageContext,
    ledger: &mut PlanRepairLedger,
) {
    let fallback_url = context.fallback_search_url();
    if fallback_url.is_empty() || is_blocked_search_engine(&fallback_url) {
        return;
    }
    let fallback_condition = build_url_wait_condition(&fallback_url);
    for step in plan.steps.iter_mut() {
        match &mut step.tool.kind {
            AgentToolKind::Wait { condition } => {
                if wait_condition_targets_blocked_search(condition) {
                    *condition = fallback_condition.clone();
                    ledger.record_note(format!("Wait condition retargeted to {}", fallback_url));
                }
            }
            AgentToolKind::Custom { name, payload }
                if name.eq_ignore_ascii_case("wait-for-condition") =>
            {
                if wait_for_condition_payload_targets_blocked_search(payload)
                    && retarget_wait_for_condition_payload(payload, &fallback_condition)
                {
                    ledger
                        .record_note(format!("wait-for-condition retargeted to {}", fallback_url));
                }
            }
            _ => {}
        }
    }
}

fn ensure_observe_validations(step: &mut AgentPlanStep, url: &str, ledger: &mut PlanRepairLedger) {
    if step.validations.iter().any(|v| {
        matches!(
            v.condition,
            AgentWaitCondition::UrlMatches(_) | AgentWaitCondition::UrlEquals(_)
        )
    }) {
        return;
    }
    let condition = build_url_wait_condition(url);
    step.validations.push(AgentValidation {
        description: format!("等待跳转至 {url}"),
        condition,
    });
    step.validations.push(AgentValidation {
        description: "等待结果列表出现".to_string(),
        condition: AgentWaitCondition::ElementVisible(AgentLocator::Css(
            "div#content_left".to_string(),
        )),
    });
    let mut overlay = stage_overlay(
        PlanStageKind::Observe,
        "search_wait",
        "wait",
        "⏱️ 等待搜索结果加载",
    );
    if let Some(obj) = overlay.as_object_mut() {
        obj.insert("step_id".to_string(), Value::String(step.id.clone()));
    }
    ledger.record_overlay(overlay);
}

fn insertion_index(plan: &AgentPlan, stage: PlanStageKind) -> usize {
    match stage {
        PlanStageKind::Navigate => 0,
        PlanStageKind::Observe => last_stage_index(plan, PlanStageKind::Act)
            .or_else(|| last_stage_index(plan, PlanStageKind::Navigate))
            .map(|idx| idx + 1)
            .unwrap_or(plan.steps.len()),
        PlanStageKind::Validate => last_stage_index(plan, PlanStageKind::Observe)
            .or_else(|| last_stage_index(plan, PlanStageKind::Act))
            .or_else(|| last_stage_index(plan, PlanStageKind::Navigate))
            .map(|idx| idx + 1)
            .unwrap_or(plan.steps.len()),
        PlanStageKind::Act => browser_search_index(plan)
            .or_else(|| last_stage_index(plan, PlanStageKind::Navigate))
            .map(|idx| idx + 1)
            .unwrap_or(plan.steps.len()),
        PlanStageKind::Evaluate => last_stage_index(plan, PlanStageKind::Observe)
            .or_else(|| last_stage_index(plan, PlanStageKind::Act))
            .map(|idx| idx + 1)
            .unwrap_or(plan.steps.len()),
        PlanStageKind::Parse => last_stage_index(plan, PlanStageKind::Validate)
            .or_else(|| last_stage_index(plan, PlanStageKind::Evaluate))
            .or_else(|| last_stage_index(plan, PlanStageKind::Observe))
            .or_else(|| last_stage_index(plan, PlanStageKind::Act))
            .map(|idx| idx + 1)
            .unwrap_or(plan.steps.len()),
        PlanStageKind::Deliver => plan.steps.len(),
    }
}

fn last_stage_index(plan: &AgentPlan, stage: PlanStageKind) -> Option<usize> {
    plan.steps
        .iter()
        .enumerate()
        .rev()
        .find(|(_, step)| classify_step(step).contains(&stage))
        .map(|(idx, _)| idx)
}

fn browser_search_index(plan: &AgentPlan) -> Option<usize> {
    plan.steps
        .iter()
        .enumerate()
        .find_map(|(idx, step)| match &step.tool.kind {
            AgentToolKind::Custom { name, .. } if name.eq_ignore_ascii_case("browser.search") => {
                Some(idx)
            }
            _ => None,
        })
}

fn mark_step_repaired(step: &mut AgentPlanStep, note: &str) {
    step.metadata
        .insert("repaired".to_string(), Value::Bool(true));
    step.metadata
        .entry("repair_notes".to_string())
        .and_modify(|value| {
            if let Value::Array(items) = value {
                items.push(Value::String(note.to_string()));
            } else {
                *value = Value::Array(vec![Value::String(note.to_string())]);
            }
        })
        .or_insert_with(|| Value::Array(vec![Value::String(note.to_string())]));
}
fn attach_repair_metadata(plan: &mut AgentPlan, report: &PlanRepairReport) {
    if !report.has_repairs() {
        return;
    }
    plan.meta.vendor_context.insert(
        "plan_repairs".to_string(),
        json!({
            "count": report.total_repairs,
            "notes": report.notes.clone(),
            "budget_exhausted": report.budget_exhausted,
        }),
    );
    plan.meta
        .vendor_context
        .insert("auto_repaired".to_string(), Value::Bool(true));
    if !report.overlays.is_empty() {
        plan.meta.overlays.extend(report.overlays.clone());
    }
}

fn repair_summary(report: &PlanRepairReport) -> Option<String> {
    if !report.has_repairs() {
        return None;
    }
    let mut preview: Vec<String> = report.notes.iter().take(3).cloned().collect();
    if report.notes.len() > 3 {
        preview.push("…".to_string());
    }
    Some(format!(
        "Auto-fixes applied ({}): {}",
        report.total_repairs,
        if preview.is_empty() {
            "details logged".to_string()
        } else {
            preview.join(" | ")
        }
    ))
}

fn append_plan_repair_note(plan: &mut AgentPlan, note: &str) {
    let trimmed = note.trim();
    if trimmed.is_empty() {
        return;
    }
    let entry = plan
        .meta
        .vendor_context
        .entry("plan_repairs".to_string())
        .or_insert_with(|| json!({ "count": 0, "notes": [] }));
    if let Some(obj) = entry.as_object_mut() {
        let notes_entry = obj
            .entry("notes".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(notes) = notes_entry {
            notes.push(Value::String(trimmed.to_string()));
        } else {
            *notes_entry = Value::Array(vec![Value::String(trimmed.to_string())]);
        }
        obj.insert(
            "last_failure_summary".to_string(),
            Value::String(trimmed.to_string()),
        );
    }
}

fn shim_unsupported_custom_tools(plan: &mut AgentPlan, ledger: &mut PlanRepairLedger) -> usize {
    let mut updates = 0;
    for step in plan.steps.iter_mut() {
        let AgentToolKind::Custom { name, .. } = &mut step.tool.kind else {
            continue;
        };
        if is_allowed_custom_tool(name) {
            continue;
        }
        let original = name.clone();
        let shimmed = plugin_shim_name(&original);
        *name = shimmed.clone();
        ledger.mark_step(
            step,
            format!(
                "Unsupported custom tool '{}' remapped to '{}'",
                original, shimmed
            ),
        );
        updates += 1;
    }
    updates
}

fn plugin_shim_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "plugin.unknown".to_string();
    }
    let mut slug = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() || ch == '-' || ch == '_' || ch == '.' {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("shim");
    }
    format!("plugin.{}", slug)
}

fn normalize_plan(plan: &mut AgentPlan, request: &AgentRequest) -> PlanRepairReport {
    let mut ledger = PlanRepairLedger::new(PLAN_REPAIR_NOTE_BUDGET);
    normalize_custom_tools_with_handler(plan, |step, note| {
        ledger.mark_step(step, note);
    });
    shim_unsupported_custom_tools(plan, &mut ledger);
    let stage_context = ContextResolver::new(request).build();
    let force_deterministic = false;
    StageAuditor::new(
        plan,
        request,
        stage_context.clone(),
        &mut ledger,
        force_deterministic,
    )
    .audit();
    ensure_weather_macro_step(plan, request, &stage_context, &mut ledger);
    ensure_click_validations(plan, &stage_context, &mut ledger);
    ensure_browser_search_payloads(plan, &stage_context, &mut ledger);
    ensure_structured_output_deliveries(plan, request, &mut ledger);
    ensure_github_repo_usernames(plan, request, &mut ledger);
    remove_empty_navigate_steps(plan, &mut ledger);
    prune_weather_navigation(plan, request, &mut ledger);
    prune_weather_followup_steps(plan, &stage_context, &mut ledger);
    retarget_wait_tools(plan, &stage_context, &mut ledger);
    auto_fill_deliver_schema(plan, &mut ledger);
    auto_fill_deliver_metadata(plan, &mut ledger);
    auto_insert_generic_parse(plan, &mut ledger);
    auto_insert_weather_parse(plan, request, &mut ledger);
    ensure_user_result_step(plan, request, &mut ledger);

    let report = ledger.into_report();
    record_auto_repair_events("plan_normalize", report.total_repairs as u64);
    report
}

fn apply_execution_tweaks(plan: &mut AgentPlan) {
    const MIN_NAV_TIMEOUT_MS: u64 = 30_000;
    let mut expect_fresh_type_wait = false;

    for step in plan.steps.iter_mut() {
        match &step.tool.kind {
            AgentToolKind::Navigate { .. } => {
                if step
                    .tool
                    .timeout_ms
                    .map(|ms| ms < MIN_NAV_TIMEOUT_MS)
                    .unwrap_or(true)
                {
                    step.tool.timeout_ms = Some(MIN_NAV_TIMEOUT_MS);
                }
                if matches!(step.tool.wait, WaitMode::Idle) {
                    step.tool.wait = WaitMode::DomReady;
                }
                expect_fresh_type_wait = true;
            }
            AgentToolKind::TypeText { .. } => {
                if expect_fresh_type_wait && matches!(step.tool.wait, WaitMode::None) {
                    step.tool.wait = WaitMode::DomReady;
                }
                expect_fresh_type_wait = false;
            }
            AgentToolKind::Click { .. }
            | AgentToolKind::Wait { .. }
            | AgentToolKind::Select { .. }
            | AgentToolKind::Scroll { .. } => {
                expect_fresh_type_wait = false;
            }
            _ => {}
        }
    }
}

fn ensure_user_result_step(
    plan: &mut AgentPlan,
    request: &AgentRequest,
    ledger: &mut PlanRepairLedger,
) -> usize {
    let needs_result = matches!(request.intent.intent_kind, AgentIntentKind::Informational)
        || requires_user_facing_result(request);
    if !needs_result {
        return 0;
    }
    if plan_has_note_step(plan) {
        return 0;
    }

    let mut note_step = build_auto_note_step(plan, request);
    ledger.mark_step(&mut note_step, "Appended agent.note for user-facing answer");
    plan.steps.push(note_step);
    1
}

fn ensure_structured_output_deliveries(
    plan: &mut AgentPlan,
    request: &AgentRequest,
    ledger: &mut PlanRepairLedger,
) -> usize {
    if request.intent.required_outputs.is_empty() {
        return 0;
    }
    let mut updates = 0;
    for output in &request.intent.required_outputs {
        let Some(schema) = normalized_schema_name(&output.schema) else {
            continue;
        };
        if plan
            .steps
            .iter()
            .any(|step| deliver_has_schema(step, &schema))
        {
            continue;
        }
        let Some((obs_index, obs_id)) = previous_observation_step(plan, plan.steps.len()) else {
            continue;
        };
        let parse_id = insert_auto_parse(plan, obs_index, &obs_id, &schema, ledger);
        let mut deliver_step = AgentPlanStep {
            id: unique_step_id(plan, &format!("deliver-{}", schema)),
            title: "交付结构化数据".to_string(),
            detail: format!("自动交付 {} 结果", schema),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.deliver.structured".to_string(),
                    payload: json!({
                        "schema": schema,
                        "artifact_label": format!("structured.{}", schema),
                        "filename": format!("{}.json", schema),
                        "source_step_id": parse_id,
                    }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(4_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        };
        ledger.mark_step(
            &mut deliver_step,
            format!("Inserted deliver step for schema {}", schema),
        );
        plan.steps.push(deliver_step);
        updates += 2;
    }
    updates
}

fn deliver_has_schema(step: &AgentPlanStep, schema: &str) -> bool {
    deliver_payload_ref(step)
        .and_then(|payload| payload.get("schema"))
        .and_then(Value::as_str)
        .map(|raw| {
            raw.trim()
                .trim_end_matches(".json")
                .eq_ignore_ascii_case(schema)
        })
        .unwrap_or(false)
}

fn normalized_schema_name(input: &str) -> Option<String> {
    let trimmed = input.trim().trim_end_matches(".json");
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_ascii_lowercase())
    }
}

fn build_auto_note_step(plan: &AgentPlan, request: &AgentRequest) -> AgentPlanStep {
    let summary = request
        .intent
        .primary_goal
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| request.goal.as_str())
        .trim()
        .to_string();

    AgentPlanStep {
        id: unique_step_id(plan, "agent-note"),
        title: "总结结果".to_string(),
        detail: "自动插入的 agent.note，用于向用户返回可读答案".to_string(),
        tool: AgentTool {
            kind: AgentToolKind::Custom {
                name: "agent.note".to_string(),
                payload: json!({
                    "title": "自动总结",
                    "detail": summary,
                }),
            },
            wait: WaitMode::None,
            timeout_ms: Some(2_000),
        },
        validations: Vec::new(),
        requires_approval: false,
        metadata: HashMap::new(),
    }
}

fn ensure_github_repo_usernames(
    plan: &mut AgentPlan,
    request: &AgentRequest,
    ledger: &mut PlanRepairLedger,
) -> usize {
    let mut updates = 0;
    for idx in 0..plan.steps.len() {
        let needs_fill = matches!(
            plan.steps[idx].tool.kind,
            AgentToolKind::Custom {
                ref name,
                ref payload,
            } if name.eq_ignore_ascii_case("data.parse.github-repo") && !payload_has_username(payload)
        );
        if !needs_fill {
            continue;
        }
        if let Some(username) = infer_github_username_for_step(plan, idx, request) {
            if let AgentToolKind::Custom { payload, .. } = &mut plan.steps[idx].tool.kind {
                let map = ensure_object(payload);
                map.insert("username".to_string(), Value::String(username.clone()));
                ledger.mark_step(
                    &mut plan.steps[idx],
                    format!("Filled missing GitHub username '{}'", username),
                );
                updates += 1;
            }
        }
    }
    updates
}

fn remove_empty_navigate_steps(plan: &mut AgentPlan, ledger: &mut PlanRepairLedger) -> usize {
    let mut removed_ids = Vec::new();
    plan.steps.retain(|step| {
        if let AgentToolKind::Navigate { url } = &step.tool.kind {
            if url.trim().is_empty() {
                removed_ids.push(step.id.clone());
                return false;
            }
        }
        true
    });
    for id in removed_ids.iter() {
        ledger.record_note(format!("Removed navigate step '{}' with empty URL", id));
    }
    removed_ids.len()
}

fn prune_weather_navigation(
    plan: &mut AgentPlan,
    request: &AgentRequest,
    ledger: &mut PlanRepairLedger,
) -> usize {
    if !requires_weather_pipeline(request) {
        return 0;
    }
    let mut removed = 0;
    if plan_has_weather_macro(plan) {
        let mut idx = 0;
        while idx < plan.steps.len() {
            if matches!(plan.steps[idx].tool.kind, AgentToolKind::Navigate { .. }) {
                let removed_step = plan.steps.remove(idx);
                ledger.record_note(format!(
                    "Removed legacy navigate '{}' in favor of weather.search",
                    removed_step.id
                ));
                removed += 1;
            } else {
                idx += 1;
            }
        }
        return removed;
    }

    let mut remove_indices = Vec::new();
    let mut seen_nav = 0;
    for (idx, step) in plan.steps.iter().enumerate() {
        if matches!(step.tool.kind, AgentToolKind::Navigate { .. }) {
            if seen_nav == 0 {
                seen_nav += 1;
            } else {
                remove_indices.push(idx);
            }
        }
    }
    if remove_indices.is_empty() {
        return 0;
    }
    for idx in remove_indices.iter().rev() {
        if *idx < plan.steps.len() {
            let removed = plan.steps.remove(*idx);
            ledger.record_note(format!(
                "Removed redundant weather navigation '{}'",
                removed.id
            ));
        }
    }
    remove_indices.len()
}

fn prune_weather_followup_steps(
    plan: &mut AgentPlan,
    _context: &StageContext,
    ledger: &mut PlanRepairLedger,
) -> usize {
    if !plan_has_weather_macro(plan) {
        return 0;
    }
    let Some(macro_idx) = plan.steps.iter().position(|step| {
        matches!(
            step.tool.kind,
            AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case("weather.search")
        )
    }) else {
        return 0;
    };
    let mut removed = 0;
    let mut idx = macro_idx + 1;
    let mut observe_indices = Vec::new();
    while idx < plan.steps.len() {
        let stages = classify_step(&plan.steps[idx]);
        if stages.contains(&PlanStageKind::Parse) || stages.contains(&PlanStageKind::Deliver) {
            break;
        }
        if stages.contains(&PlanStageKind::Observe) {
            observe_indices.push(idx);
            idx += 1;
            continue;
        }
        let step = plan.steps.remove(idx);
        ledger.record_note(format!(
            "Removed redundant step '{}' after weather.search",
            step.id
        ));
        removed += 1;
    }
    if observe_indices.len() > 1 {
        // Retain the last observation before parse stage, remove earlier ones.
        for &obsolete_idx in observe_indices[..observe_indices.len() - 1].iter().rev() {
            if obsolete_idx < plan.steps.len() {
                let step = plan.steps.remove(obsolete_idx);
                ledger.record_note(format!(
                    "Removed redundant observation '{}' after weather.search",
                    step.id
                ));
                removed += 1;
            }
        }
    }
    removed
}

fn auto_fill_deliver_schema(plan: &mut AgentPlan, ledger: &mut PlanRepairLedger) -> usize {
    let mut updates = 0;
    for idx in 0..plan.steps.len() {
        let Some(schema_to_apply) = ({
            let payload = match deliver_payload_ref(&plan.steps[idx]) {
                Some(value) => value,
                None => continue,
            };
            let has_schema = payload
                .get("schema")
                .and_then(Value::as_str)
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);
            if has_schema {
                None
            } else {
                infer_schema_from_previous_parse(plan, idx)
            }
        }) else {
            continue;
        };

        if let Some(payload) = deliver_payload_map(&mut plan.steps[idx]) {
            payload.insert("schema".to_string(), Value::String(schema_to_apply.clone()));
            ledger.mark_step(
                &mut plan.steps[idx],
                format!("Auto-filled deliver schema as {}", schema_to_apply),
            );
            updates += 1;
        }
    }
    updates
}

fn auto_fill_deliver_metadata(plan: &mut AgentPlan, ledger: &mut PlanRepairLedger) -> usize {
    let mut updates = 0;
    for step in plan.steps.iter_mut() {
        let Some(payload) = deliver_payload_map(step) else {
            continue;
        };

        let schema = payload
            .get("schema")
            .and_then(Value::as_str)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        let mut changed_fields = Vec::new();
        if payload
            .get("artifact_label")
            .and_then(Value::as_str)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
            == false
        {
            if let Some(schema_value) = schema.as_ref() {
                payload.insert(
                    "artifact_label".to_string(),
                    Value::String(format!("structured.{}", schema_value)),
                );
                changed_fields.push("artifact_label");
            }
        }

        if payload
            .get("filename")
            .and_then(Value::as_str)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
            == false
        {
            if let Some(schema_part) = schema.as_deref() {
                let filename = format!("{}.json", schema_part);
                payload.insert("filename".to_string(), Value::String(filename));
                changed_fields.push("filename");
            }
        }

        if !changed_fields.is_empty() {
            ledger.mark_step(
                step,
                format!("Auto-filled deliver {}", changed_fields.join("/")),
            );
            updates += 1;
        }
    }
    updates
}

fn deliver_payload_map(step: &mut AgentPlanStep) -> Option<&mut Map<String, Value>> {
    match &mut step.tool.kind {
        AgentToolKind::Custom { name, payload }
            if name.eq_ignore_ascii_case("data.deliver.structured") =>
        {
            Some(ensure_object(payload))
        }
        _ => None,
    }
}

fn deliver_payload_ref(step: &AgentPlanStep) -> Option<&Map<String, Value>> {
    match &step.tool.kind {
        AgentToolKind::Custom { name, payload }
            if name.eq_ignore_ascii_case("data.deliver.structured") =>
        {
            payload.as_object()
        }
        _ => None,
    }
}

fn deliver_source_step_id(step: &AgentPlanStep) -> Option<String> {
    deliver_payload_ref(step)
        .and_then(|payload| payload.get("source_step_id"))
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

struct DeliverParseFix {
    deliver_id: String,
    observation_id: String,
    schema: String,
}

fn auto_insert_generic_parse(plan: &mut AgentPlan, ledger: &mut PlanRepairLedger) -> usize {
    let mut updates = 0;
    let mut fixes = Vec::new();
    for idx in 0..plan.steps.len() {
        let Some(schema) = deliver_payload_ref(&plan.steps[idx])
            .and_then(|payload| payload.get("schema"))
            .and_then(Value::as_str)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| Some("generic_observation_v1".to_string()))
        else {
            continue;
        };

        let Some(step_id) = deliver_source_step_id(&plan.steps[idx]) else {
            if let Some((_parse_index, parse_id)) = previous_parse_step(plan, idx) {
                let deliver_id = plan.steps[idx].id.clone();
                if let Some(payload) = deliver_payload_map(&mut plan.steps[idx]) {
                    payload.insert(
                        "source_step_id".to_string(),
                        Value::String(parse_id.clone()),
                    );
                    ledger.mark_step(
                        &mut plan.steps[idx],
                        format!("Linked deliver '{}' to parser '{}'", deliver_id, parse_id),
                    );
                    updates += 1;
                }
                continue;
            }
            if let Some((_obs_index, obs_id)) = previous_observation_step(plan, idx) {
                fixes.push(DeliverParseFix {
                    deliver_id: plan.steps[idx].id.clone(),
                    observation_id: obs_id,
                    schema: schema.clone(),
                });
            }
            continue;
        };

        let Some(source_index) = plan
            .steps
            .iter()
            .position(|candidate| candidate.id == step_id)
        else {
            continue;
        };
        if is_parse_step(&plan.steps[source_index]) {
            continue;
        }
        if !is_observation_step(&plan.steps[source_index]) {
            continue;
        }
        fixes.push(DeliverParseFix {
            deliver_id: plan.steps[idx].id.clone(),
            observation_id: plan.steps[source_index].id.clone(),
            schema: schema.clone(),
        });
    }

    if fixes.is_empty() {
        return updates;
    }

    let mut observation_cache: HashMap<String, String> = HashMap::new();
    for fix in fixes {
        let Some(obs_index) = plan
            .steps
            .iter()
            .position(|step| step.id == fix.observation_id)
        else {
            continue;
        };
        let parse_step_id = observation_cache
            .entry(fix.observation_id.clone())
            .or_insert_with(|| {
                insert_auto_parse(plan, obs_index, &fix.observation_id, &fix.schema, ledger)
            })
            .clone();

        if let Some(deliver_step) = plan.steps.iter_mut().find(|step| step.id == fix.deliver_id) {
            if let Some(payload) = deliver_payload_map(deliver_step) {
                payload.insert(
                    "source_step_id".to_string(),
                    Value::String(parse_step_id.clone()),
                );
                ledger.mark_step(
                    deliver_step,
                    format!(
                        "Linked deliver '{}' to parser '{}'",
                        deliver_step.id, parse_step_id
                    ),
                );
                updates += 1;
            }
        }
    }

    updates
}

fn auto_insert_weather_parse(
    plan: &mut AgentPlan,
    request: &AgentRequest,
    ledger: &mut PlanRepairLedger,
) -> usize {
    if !requires_weather_pipeline(request) {
        return 0;
    }
    let mut updates = 0;
    let mut inserted_pipeline = false;
    let parse_step_id = if let Some((idx, _)) =
        plan.steps
            .iter()
            .enumerate()
            .find(|(_, step)| match &step.tool.kind {
                AgentToolKind::Custom { name, .. } => {
                    name.eq_ignore_ascii_case("data.parse.weather")
                }
                _ => false,
            }) {
        plan.steps[idx].id.clone()
    } else {
        let Some((observation_index, observation_id)) =
            previous_observation_step(plan, plan.steps.len())
        else {
            return 0;
        };

        let mut parse_step = AgentPlanStep {
            id: unique_step_id(plan, &format!("{}-weather-parse", observation_id)),
            title: "解析天气数据".to_string(),
            detail: "自动插入的 data.parse.weather，用于满足天气查询".to_string(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.parse.weather".to_string(),
                    payload: json!({
                        "source_step_id": observation_id,
                        "title": "Auto parse weather",
                        "detail": "Synthesized weather parser"
                    }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(8_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        };
        let parse_id = parse_step.id.clone();
        ledger.mark_step(&mut parse_step, "Inserted weather parser");
        plan.steps.insert(observation_index + 1, parse_step);
        updates += 1;
        inserted_pipeline = true;
        parse_id
    };

    let mut found_deliver = false;
    for step in plan.steps.iter_mut() {
        #[cfg(test)]
        let _step_id_dbg = step.id.clone();
        let Some(payload) = deliver_payload_map(step) else {
            continue;
        };
        let schema_name = payload
            .get("schema")
            .and_then(Value::as_str)
            .map(|value| value.trim().trim_end_matches(".json").to_ascii_lowercase());
        if matches!(schema_name.as_deref(), Some(schema) if schema == "weather_report_v1") {
            retarget_deliver_to_weather(payload, &parse_step_id);
            let _ = payload;
            ledger.mark_step(
                step,
                format!(
                    "Linked weather deliver '{}' to parser {}",
                    step.id, parse_step_id
                ),
            );
            let mut overlay = stage_overlay(
                PlanStageKind::Deliver,
                "weather_align",
                "adjust",
                "🌦️ 校准天气交付",
            );
            if let Some(obj) = overlay.as_object_mut() {
                obj.insert("step_id".to_string(), Value::String(step.id.clone()));
            }
            ledger.record_overlay(overlay);
            updates += 1;
            found_deliver = true;
            break;
        }
        if !found_deliver {
            retarget_deliver_to_weather(payload, &parse_step_id);
            let _ = payload;
            ledger.mark_step(
                step,
                format!("Retargeted deliver '{}' to weather schema", step.id),
            );
            let mut overlay = stage_overlay(
                PlanStageKind::Deliver,
                "weather_adjust",
                "adjust",
                "🌦️ 调整交付为 weather_report_v1",
            );
            if let Some(obj) = overlay.as_object_mut() {
                obj.insert("step_id".to_string(), Value::String(step.id.clone()));
            }
            ledger.record_overlay(overlay);
            updates += 1;
            found_deliver = true;
            break;
        }
    }

    if !found_deliver {
        let mut deliver_step = AgentPlanStep {
            id: unique_step_id(plan, "deliver-weather"),
            title: "交付天气数据".to_string(),
            detail: "自动插入的 data.deliver.structured，用于天气报告".to_string(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.deliver.structured".to_string(),
                    payload: json!({
                        "schema": "weather_report_v1",
                        "artifact_label": "structured.weather_report_v1",
                        "filename": "weather_report_v1.json",
                        "source_step_id": parse_step_id,
                    }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(4_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        };
        ledger.mark_step(
            &mut deliver_step,
            "Inserted deliver step for weather report",
        );
        plan.steps.push(deliver_step);
        updates += 1;
        inserted_pipeline = true;
    }

    if inserted_pipeline {
        ledger.record_overlay(json!({
            "kind": "repair.weather_pipeline",
            "title": "已自动补齐天气流水线",
            "detail": format!(
                "已追加 data.parse.weather / data.deliver.structured 以满足 {}",
                weather_query_text(request)
            ),
        }));
    }

    updates + prune_duplicate_weather_deliver(plan, ledger)
}

fn retarget_deliver_to_weather(payload: &mut Map<String, Value>, parse_step_id: &str) {
    payload.insert(
        "schema".to_string(),
        Value::String("weather_report_v1".to_string()),
    );
    payload.insert(
        "artifact_label".to_string(),
        Value::String("structured.weather_report_v1".to_string()),
    );
    payload.insert(
        "filename".to_string(),
        Value::String("weather_report_v1.json".to_string()),
    );
    payload.insert(
        "source_step_id".to_string(),
        Value::String(parse_step_id.to_string()),
    );
}

fn prune_duplicate_weather_deliver(plan: &mut AgentPlan, ledger: &mut PlanRepairLedger) -> usize {
    let mut seen_primary = false;
    let mut remove_indices = Vec::new();
    for (idx, step) in plan.steps.iter().enumerate() {
        let Some(payload) = deliver_payload_ref(step) else {
            continue;
        };
        if is_weather_schema(payload) {
            if seen_primary {
                remove_indices.push(idx);
            } else {
                seen_primary = true;
            }
        }
    }

    if remove_indices.is_empty() {
        return 0;
    }

    let removed_count = remove_indices.len();
    for idx in remove_indices.into_iter().rev() {
        let removed = plan.steps.remove(idx);
        ledger.record_note(format!(
            "Removed duplicate weather deliver '{}'",
            removed.id
        ));
        let mut overlay = stage_overlay(
            PlanStageKind::Deliver,
            "weather_dedup",
            "cleanup",
            "♻️ 已去重天气交付",
        );
        if let Some(obj) = overlay.as_object_mut() {
            obj.insert("step_id".to_string(), Value::String(removed.id));
        }
        ledger.record_overlay(overlay);
    }
    removed_count
}

fn is_weather_schema(payload: &Map<String, Value>) -> bool {
    payload
        .get("schema")
        .and_then(Value::as_str)
        .map(|value| {
            value
                .trim()
                .trim_end_matches(".json")
                .eq_ignore_ascii_case("weather_report_v1")
        })
        .unwrap_or(false)
}

/// Check if plan has a custom tool with the given name (case-insensitive)
fn plan_has_custom_tool(plan: &AgentPlan, tool_name: &str) -> bool {
    plan.steps.iter().any(|step| {
        matches!(
            &step.tool.kind,
            AgentToolKind::Custom { name, .. } if name.eq_ignore_ascii_case(tool_name)
        )
    })
}

/// Check if plan has a custom tool matching a predicate
fn plan_has_custom_tool_matching<F: Fn(&str) -> bool>(plan: &AgentPlan, predicate: F) -> bool {
    plan.steps.iter().any(|step| {
        matches!(
            &step.tool.kind,
            AgentToolKind::Custom { name, .. } if predicate(name)
        )
    })
}

fn plan_has_deliver_step(plan: &AgentPlan) -> bool {
    plan_has_custom_tool(plan, "data.deliver.structured")
}
fn plan_has_auto_act(plan: &AgentPlan) -> bool {
    plan.meta.vendor_context.contains_key("auto_act_engine")
}
fn plan_has_extract_site(plan: &AgentPlan) -> bool {
    plan_has_custom_tool(plan, "data.extract-site")
}
fn plan_has_target_validation(plan: &AgentPlan) -> bool {
    plan_has_custom_tool(plan, "data.validate-target")
}
fn plan_has_browser_search(plan: &AgentPlan) -> bool {
    plan_has_custom_tool(plan, "browser.search")
}
fn plan_has_note_step(plan: &AgentPlan) -> bool {
    plan_has_custom_tool(plan, "agent.note")
}
fn plan_has_weather_macro(plan: &AgentPlan) -> bool {
    plan_has_custom_tool(plan, "weather.search")
}
fn plan_has_parse_step(plan: &AgentPlan) -> bool {
    plan_has_custom_tool_matching(plan, |n| {
        n.starts_with("data.parse.") || n.eq_ignore_ascii_case("market.quote.fetch")
    })
}
fn plan_has_navigate_step(plan: &AgentPlan) -> bool {
    plan.steps
        .iter()
        .any(|step| matches!(step.tool.kind, AgentToolKind::Navigate { .. }))
}
fn plan_has_deliver_stage(plan: &AgentPlan) -> bool {
    plan_has_deliver_step(plan) || plan_has_note_step(plan)
}
fn plan_has_observation_step(plan: &AgentPlan) -> bool {
    plan.steps.iter().any(is_observation_step)
}

fn validation_covers_navigation(validation: &AgentValidation) -> bool {
    matches!(
        validation.condition,
        AgentWaitCondition::UrlMatches(_) | AgentWaitCondition::UrlEquals(_)
    )
}

fn ensure_click_validations(
    plan: &mut AgentPlan,
    context: &StageContext,
    ledger: &mut PlanRepairLedger,
) {
    let fallback_url = context
        .best_known_url()
        .unwrap_or_else(|| context.fallback_search_url());
    for step in plan.steps.iter_mut() {
        let AgentToolKind::Click { .. } = &step.tool.kind else {
            continue;
        };
        if step
            .metadata
            .get(SKIP_CLICK_VALIDATION_METADATA_KEY)
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let explicit_expectation = step
            .metadata
            .get(EXPECTED_URL_METADATA_KEY)
            .and_then(Value::as_str)
            .and_then(click_expectation_from_hint);
        let (target_url, condition) = explicit_expectation
            .or_else(|| inferred_click_expectation(step))
            .map(|expectation| match expectation {
                ClickExpectation::Absolute(url) => {
                    let domain_only = domain_from_url(&url).unwrap_or(url.clone());
                    let pattern = build_domain_match_pattern(&domain_only);
                    (url, AgentWaitCondition::UrlMatches(pattern))
                }
                ClickExpectation::Domain(domain) => {
                    let pattern = build_domain_match_pattern(&domain);
                    let url = normalize_domain_hint_to_url(&domain);
                    (url, AgentWaitCondition::UrlMatches(pattern))
                }
            })
            .unwrap_or_else(|| {
                let url = fallback_url.clone();
                let domain_only = domain_from_url(&url).unwrap_or(url.clone());
                let pattern = build_domain_match_pattern(&domain_only);
                (url, AgentWaitCondition::UrlMatches(pattern))
            });
        let description = format!("自动等待跳转至 {target_url}");

        let mut reused = false;
        for validation in step.validations.iter_mut() {
            if validation_covers_navigation(validation) {
                validation.description = description.clone();
                validation.condition = condition.clone();
                reused = true;
                break;
            }
        }

        if !reused {
            step.validations.push(AgentValidation {
                description: description.clone(),
                condition: condition.clone(),
            });
        }

        if !step.metadata.contains_key(EXPECTED_URL_METADATA_KEY) {
            step.metadata.insert(
                EXPECTED_URL_METADATA_KEY.to_string(),
                Value::String(target_url.clone()),
            );
        }
        ledger.mark_step(
            step,
            format!("Auto-added click validation targeting {target_url}"),
        );
        let mut overlay = stage_overlay(
            PlanStageKind::Act,
            "click_validation",
            "adjust",
            "🔁 自动补齐点击跳转校验",
        );
        if let Some(obj) = overlay.as_object_mut() {
            obj.insert("target".to_string(), Value::String(target_url.clone()));
            obj.insert("step_id".to_string(), Value::String(step.id.clone()));
        }
        ledger.record_overlay(overlay);
    }
}

fn ensure_browser_search_payloads(
    plan: &mut AgentPlan,
    context: &StageContext,
    ledger: &mut PlanRepairLedger,
) {
    let fallback_query = context.search_seed();
    let site_hint = context.preferred_sites.first().cloned();

    for step in plan.steps.iter_mut() {
        let AgentToolKind::Custom { name, payload } = &mut step.tool.kind else {
            continue;
        };
        if !name.eq_ignore_ascii_case("browser.search") {
            continue;
        }
        let mut query_note: Option<String> = None;
        let mut site_note: Option<String> = None;

        if !payload.is_object() {
            *payload = json!({});
        }
        {
            let Some(obj) = payload.as_object_mut() else {
                continue;
            };
            let missing_query = obj
                .get("query")
                .and_then(Value::as_str)
                .map(|value| value.trim().is_empty())
                .unwrap_or(true);
            if missing_query {
                obj.insert("query".to_string(), Value::String(fallback_query.clone()));
                query_note = Some(format!(
                    "自动补全 browser.search 查询词：{}",
                    fallback_query
                ));
            }
            if obj.get("site").is_none() {
                if let Some(site) = site_hint.clone() {
                    obj.insert("site".to_string(), Value::String(site.clone()));
                    site_note = Some(format!("为 browser.search 添加站点限定：{}", site));
                }
            }
        }

        if let Some(note) = query_note {
            ledger.mark_step(step, note);
        }
        if let Some(note) = site_note {
            ledger.mark_step(step, note);
        }
    }
}

fn ensure_weather_macro_step(
    plan: &mut AgentPlan,
    request: &AgentRequest,
    context: &StageContext,
    ledger: &mut PlanRepairLedger,
) -> usize {
    if !requires_weather_pipeline(request) {
        return 0;
    }
    if plan_has_weather_macro(plan) {
        return 0;
    }
    let query = context
        .search_terms
        .first()
        .cloned()
        .unwrap_or_else(|| request.goal.clone());
    let mut step = AgentPlanStep {
        id: unique_step_id(plan, "weather-search"),
        title: "天气搜索".to_string(),
        detail: "自动插入 weather.search 宏工具".to_string(),
        tool: AgentTool {
            kind: AgentToolKind::Custom {
                name: "weather.search".to_string(),
                payload: json!({
                    "query": query,
                    "result_selector": "div#content_left"
                }),
            },
            wait: WaitMode::None,
            timeout_ms: Some(30_000),
        },
        validations: Vec::new(),
        requires_approval: false,
        metadata: HashMap::new(),
    };
    ledger.mark_step(&mut step, "确保天气搜索使用宏工具");
    plan.steps.insert(0, step);
    1
}

fn build_url_wait_condition(url: &str) -> AgentWaitCondition {
    match Url::parse(url) {
        Ok(parsed) => {
            let mut base = format!(
                "{}://{}",
                parsed.scheme(),
                parsed.host_str().unwrap_or_default()
            );
            if let Some(port) = parsed.port() {
                base.push(':');
                base.push_str(&port.to_string());
            }
            base.push_str(parsed.path());

            if let Some((_, value)) = parsed.query_pairs().find(|(key, _)| key == "wd") {
                let encoded: String = form_urlencoded::byte_serialize(value.as_bytes()).collect();
                let mut pattern = format!("^{}.*", escape(&base));
                pattern.push_str(&format!("wd={}.*", escape(&encoded)));
                pattern.push('$');
                AgentWaitCondition::UrlMatches(pattern)
            } else {
                AgentWaitCondition::UrlEquals(parsed.into())
            }
        }
        Err(_) => AgentWaitCondition::UrlEquals(url.to_string()),
    }
}

fn build_domain_match_pattern(domain: &str) -> String {
    let trimmed = domain
        .trim()
        .trim_start_matches("*")
        .trim_matches('/')
        .trim_start_matches(".");
    if trimmed.is_empty() {
        return String::from(".*");
    }
    let escaped_domain = escape(trimmed);
    format!(r"^https?://[^/]*{escaped_domain}.*$")
}

fn domain_from_url(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_string()))
}

fn normalize_domain_hint_to_url(domain: &str) -> String {
    let trimmed = domain.trim().trim_start_matches("*").trim_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if trimmed.starts_with("//") {
        format!("https:{}", trimmed)
    } else {
        format!("https://{}", trimmed)
    }
}

#[derive(Debug, Clone)]
enum ClickExpectation {
    Absolute(String),
    Domain(String),
}

fn click_expectation_from_hint(hint: &str) -> Option<ClickExpectation> {
    let trimmed = hint.trim();
    if trimmed.is_empty() {
        None
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Some(ClickExpectation::Absolute(trimmed.to_string()))
    } else if trimmed.starts_with("//") {
        Some(ClickExpectation::Absolute(format!("https:{}", trimmed)))
    } else {
        Some(ClickExpectation::Domain(trimmed.to_string()))
    }
}

fn inferred_click_expectation(step: &AgentPlanStep) -> Option<ClickExpectation> {
    match &step.tool.kind {
        AgentToolKind::Click { locator } => match locator {
            AgentLocator::Css(selector) => infer_expectation_from_css(selector),
            _ => None,
        },
        _ => None,
    }
}

fn infer_expectation_from_css(selector: &str) -> Option<ClickExpectation> {
    extract_href_value(selector).and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            Some(ClickExpectation::Absolute(trimmed.to_string()))
        } else if trimmed.starts_with("//") {
            Some(ClickExpectation::Absolute(format!("https:{}", trimmed)))
        } else if trimmed.contains('.') {
            Some(ClickExpectation::Domain(trimmed.to_string()))
        } else {
            None
        }
    })
}

fn extract_href_value(selector: &str) -> Option<String> {
    let lower = selector.to_ascii_lowercase();
    let idx = lower.find("href")?;
    let bytes = selector.as_bytes();
    let mut i = idx + 4;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'*' {
        i += 1;
    }
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'=' {
        return None;
    }
    i += 1;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let quote = bytes[i];
    if quote == b'"' || quote == b'\'' {
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        return Some(selector[start..i].to_string());
    }
    let start = i;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b']' {
        i += 1;
    }
    if start == i {
        None
    } else {
        Some(selector[start..i].to_string())
    }
}

fn previous_observation_step(plan: &AgentPlan, end_index: usize) -> Option<(usize, String)> {
    plan.steps
        .iter()
        .take(end_index)
        .enumerate()
        .rev()
        .find_map(|(idx, step)| {
            if is_observation_step(step) {
                Some((idx, step.id.clone()))
            } else {
                None
            }
        })
}

fn previous_parse_step(plan: &AgentPlan, end_index: usize) -> Option<(usize, String)> {
    plan.steps
        .iter()
        .take(end_index)
        .enumerate()
        .rev()
        .find_map(|(idx, step)| {
            if is_parse_step(step) {
                Some((idx, step.id.clone()))
            } else {
                None
            }
        })
}

fn is_parse_step(step: &AgentPlanStep) -> bool {
    matches!(
        step.tool.kind,
        AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case("data.parse.generic")
            || name.eq_ignore_ascii_case("data.parse.market_info")
            || name.eq_ignore_ascii_case("data.parse.news_brief")
            || name.eq_ignore_ascii_case("data.parse.twitter-feed")
            || name.eq_ignore_ascii_case("data.parse.facebook-feed")
            || name.eq_ignore_ascii_case("data.parse.linkedin-profile")
            || name.eq_ignore_ascii_case("data.parse.hackernews-feed")
            || name.eq_ignore_ascii_case("data.parse.github-repo")
    )
}

fn is_observation_step(step: &AgentPlanStep) -> bool {
    matches!(
        step.tool.kind,
        AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case("data.extract-site")
    )
}

fn insert_auto_parse(
    plan: &mut AgentPlan,
    observation_index: usize,
    observation_id: &str,
    schema: &str,
    ledger: &mut PlanRepairLedger,
) -> String {
    let parse_step_id = unique_step_id(plan, &format!("{}-parse", observation_id));
    let mut parse_step = AgentPlanStep {
        id: parse_step_id.clone(),
        title: "自动解析结构化数据".to_string(),
        detail: "自动插入的 data.parse.generic，用于补齐 deliver 依赖".to_string(),
        tool: AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.parse.generic".to_string(),
                payload: json!({
                    "source_step_id": observation_id,
                    "schema": schema,
                    "title": "Auto parse observation",
                    "detail": format!("Synthesized parser for {schema}"),
                }),
            },
            wait: WaitMode::None,
            timeout_ms: Some(5_000),
        },
        validations: Vec::new(),
        requires_approval: false,
        metadata: HashMap::new(),
    };

    ledger.mark_step(
        &mut parse_step,
        format!("Inserted generic parser for schema {}", schema),
    );

    plan.steps.insert(observation_index + 1, parse_step);
    parse_step_id
}

#[cfg(test)]
mod deliver_autofill_tests {
    use super::*;
    use agent_core::RequestedOutput;

    fn observation_step(id: &str) -> AgentPlanStep {
        AgentPlanStep {
            id: id.to_string(),
            title: "观察页面".to_string(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.extract-site".to_string(),
                    payload: json!({}),
                },
                wait: WaitMode::None,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        }
    }

    fn deliver_step(id: &str) -> AgentPlanStep {
        AgentPlanStep {
            id: id.to_string(),
            title: "返回结构化结果".to_string(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.deliver.structured".to_string(),
                    payload: json!({ "schema": "generic_observation_v1" }),
                },
                wait: WaitMode::None,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn links_deliver_to_prior_parse_when_source_missing() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "parse-only");
        plan.steps.push(AgentPlanStep {
            id: "parse-1".into(),
            title: "解析仓库".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.parse.github-repo".into(),
                    payload: json!({ "username": "demo" }),
                },
                wait: WaitMode::None,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        });
        plan.steps.push(deliver_step("deliver-1"));

        let mut ledger = PlanRepairLedger::new(4);
        let updates = auto_insert_generic_parse(&mut plan, &mut ledger);
        assert_eq!(updates, 1);
        assert_eq!(plan.steps.len(), 2, "should not insert extra parse steps");

        let deliver_step = plan
            .steps
            .iter()
            .find(|step| step.id == "deliver-1")
            .expect("deliver step present");
        let payload = match &deliver_step.tool.kind {
            AgentToolKind::Custom { payload, .. } => payload
                .as_object()
                .expect("deliver payload should be an object"),
            other => panic!("unexpected tool kind: {:?}", other),
        };
        assert_eq!(
            payload.get("source_step_id").and_then(Value::as_str),
            Some("parse-1")
        );
    }

    #[test]
    fn auto_parse_and_metadata_fill_for_deliver() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "autofix");
        plan.steps.push(observation_step("llm-step-4"));
        plan.steps.push(deliver_step("llm-step-6"));

        let request = AgentRequest::new(task_id, "auto repair");
        let context = ContextResolver::new(&request).build();
        assert!(context.fallback_search_url().contains("baidu"));
        normalize_plan(&mut plan, &request);

        let deliver_step = plan
            .steps
            .iter()
            .find(|step| matches!(&step.tool.kind, AgentToolKind::Custom { name, .. } if name == "data.deliver.structured"))
            .expect("deliver step present");
        let payload = deliver_payload_ref(deliver_step).expect("deliver payload object");
        assert_eq!(
            payload.get("artifact_label").and_then(Value::as_str),
            Some("structured.generic_observation_v1")
        );
        assert_eq!(
            payload.get("filename").and_then(Value::as_str),
            Some("generic_observation_v1.json")
        );
        let source_id = payload
            .get("source_step_id")
            .and_then(Value::as_str)
            .expect("deliver linked to parse");
        let parse_step = plan
            .steps
            .iter()
            .find(|step| step.id == source_id)
            .expect("parse step exists for deliver");
        match &parse_step.tool.kind {
            AgentToolKind::Custom { name, payload } => {
                assert!(name.eq_ignore_ascii_case("data.parse.generic"));
                let obj = payload.as_object().expect("parse payload should be object");
                assert_eq!(
                    obj.get("schema").and_then(Value::as_str),
                    Some("generic_observation_v1")
                );
                assert_eq!(
                    obj.get("source_step_id").and_then(Value::as_str),
                    Some("llm-step-4")
                );
            }
            other => panic!("unexpected parse tool: {:?}", other),
        }
    }

    #[test]
    fn auto_appends_note_for_user_facing_prompts() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "needs-note");
        plan.steps.push(observation_step("obs-1"));

        let request = AgentRequest::new(task_id, "告诉我结果");
        let context = ContextResolver::new(&request).build();
        assert!(context.fallback_search_url().contains("baidu"));
        normalize_plan(&mut plan, &request);
        assert!(plan.steps.iter().any(|step| matches!(
            &step.tool.kind,
            AgentToolKind::Custom { name, .. } if name == "agent.note"
        )));
    }

    #[test]
    fn auto_inserts_navigation_when_target_site_known() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "needs-nav");
        plan.steps.push(observation_step("obs-1"));

        let mut request = AgentRequest::new(task_id, "collect data");
        request
            .intent
            .target_sites
            .push("https://example.com".to_string());
        normalize_plan(&mut plan, &request);

        let first_kind = plan.steps.first().map(|step| &step.tool.kind);
        assert!(
            matches!(first_kind, Some(AgentToolKind::Navigate { .. }))
                || matches!(
                    first_kind,
                    Some(AgentToolKind::Custom { name, .. }) if name == "browser.search"
                ),
            "expected navigate or browser.search at plan start, found {:?}",
            first_kind
        );
    }

    #[test]
    fn auto_inserts_navigation_with_query_fallback() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "needs-nav-fallback");
        let request = AgentRequest::new(task_id, "查询人工智能新闻");

        normalize_plan(&mut plan, &request);

        match plan.steps.first().map(|step| &step.tool.kind) {
            Some(AgentToolKind::Navigate { url }) => {
                assert!(
                    url.contains("baidu.com") || url.contains("news.google"),
                    "unexpected fallback url: {}",
                    url
                );
                assert!(url.contains("%E4%BA%BA%E5%B7%A5%E6%99%BA%E8%83%BD"));
            }
            Some(AgentToolKind::Custom { name, payload }) if name == "browser.search" => {
                let query = payload
                    .as_object()
                    .and_then(|obj| obj.get("query"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                assert!(
                    query.contains("人工智能新闻"),
                    "search payload missing query {query}"
                );
            }
            other => panic!("expected navigate/search step, got {:?}", other),
        }
    }

    #[test]
    fn auto_inserts_observation_for_structured_outputs() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "needs-observation");

        let mut request = AgentRequest::new(task_id, "collect quotes");
        request
            .intent
            .required_outputs
            .push(RequestedOutput::new("market_info_v1.json"));
        request
            .intent
            .target_sites
            .push("https://example.com".to_string());

        normalize_plan(&mut plan, &request);

        assert!(plan.steps.iter().any(|step| {
            matches!(
                &step.tool.kind,
                AgentToolKind::Custom { name, .. } if name == "data.extract-site"
            )
        }));
    }

    #[test]
    fn structured_output_pipeline_inserted() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "needs-schema");
        plan.steps.push(observation_step("obs-required"));

        let mut request = AgentRequest::new(task_id, "summarize market info");
        request
            .intent
            .required_outputs
            .push(RequestedOutput::new("market_info_v1.json"));

        let report = normalize_plan(&mut plan, &request);
        assert!(report.has_repairs());
        assert!(plan.steps.iter().any(|step| {
            matches!(
                &step.tool.kind,
                AgentToolKind::Custom { name, .. } if name.eq_ignore_ascii_case("data.deliver.structured")
            )
        }));
        assert!(plan.steps.iter().any(|step| {
            matches!(
                &step.tool.kind,
                AgentToolKind::Custom { name, .. } if name.eq_ignore_ascii_case("data.parse.generic")
            )
        }));
    }

    #[test]
    fn shims_unknown_custom_tools() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "shim-tools");
        plan.steps.push(AgentPlanStep {
            id: "llm-step-1".into(),
            title: "LLM step".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "custom.magic".into(),
                    payload: json!({}),
                },
                wait: WaitMode::None,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        });

        let request = AgentRequest::new(task_id, "run custom tool");
        let report = normalize_plan(&mut plan, &request);

        assert!(report.has_repairs());
        let plugin_step = plan
            .steps
            .iter()
            .find(|step| {
                matches!(
                    &step.tool.kind,
                    AgentToolKind::Custom { name, .. } if name.starts_with("plugin.")
                )
            })
            .expect("plugin shim step present");
        assert_eq!(
            plugin_step
                .metadata
                .get("repaired")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn attaches_plan_level_repair_metadata() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "meta-repair");
        plan.steps.push(observation_step("obs-meta"));
        plan.steps.push(deliver_step("deliver-missing"));

        let request = AgentRequest::new(task_id, "meta test");
        let report = normalize_plan(&mut plan, &request);
        attach_repair_metadata(&mut plan, &report);

        let repairs = plan
            .meta
            .vendor_context
            .get("plan_repairs")
            .and_then(Value::as_object)
            .cloned()
            .expect("plan repairs metadata");
        assert!(repairs.get("count").is_some());
        assert!(repairs.get("notes").is_some());
        assert_eq!(
            plan.meta
                .vendor_context
                .get("auto_repaired")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn auto_inserts_weather_parse_when_needed() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "weather");
        plan.steps.push(observation_step("obs-weather"));

        let request = AgentRequest::new(task_id, "请告诉我北京天气");
        normalize_plan(&mut plan, &request);

        assert!(plan.steps.iter().any(|step| match &step.tool.kind {
            AgentToolKind::Custom { name, payload }
                if name.eq_ignore_ascii_case("data.parse.weather") =>
            {
                payload
                    .as_object()
                    .and_then(|obj| obj.get("source_step_id"))
                    .and_then(Value::as_str)
                    .map(|id| id.contains("obs-weather"))
                    .unwrap_or(false)
            }
            _ => false,
        }));
        assert!(plan.steps.iter().any(|step| {
            matches!(
                &step.tool.kind,
                AgentToolKind::Custom { name, payload }
                    if name == "data.deliver.structured"
                        && payload
                            .as_object()
                            .and_then(|obj| obj.get("schema"))
                            .and_then(Value::as_str)
                            == Some("weather_report_v1")
            )
        }));
    }

    #[test]
    fn stage_auditor_recovers_missing_stages() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "demo");
        let request = AgentRequest::new(task_id, "帮我看看今天的天气");

        let report = normalize_plan(&mut plan, &request);
        assert!(report.total_repairs > 0);
        let has_navigate = plan
            .steps
            .iter()
            .any(|step| matches!(step.tool.kind, AgentToolKind::Navigate { .. }));
        let has_search = plan.steps.iter().any(|step| {
            matches!(
                &step.tool.kind,
                AgentToolKind::Custom { name, .. }
                    if name.eq_ignore_ascii_case("browser.search")
                        || name.eq_ignore_ascii_case("weather.search")
            )
        });
        assert!(has_navigate || has_search, "navigate/search stage missing");
        assert!(plan.steps.iter().any(|step| {
            matches!(
                &step.tool.kind,
                AgentToolKind::Custom { name, .. } if name == "data.extract-site"
            )
        }));
        assert!(plan.steps.iter().any(|step| {
            matches!(
                &step.tool.kind,
                AgentToolKind::Custom { name, .. } if name == "data.deliver.structured"
                    || name == "agent.note"
            )
        }));
    }

    #[test]
    fn informational_execute_observes_current_page() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "context");
        let mut request = AgentRequest::new(task_id, "抓取当前页面内容");
        request.intent.intent_kind = AgentIntentKind::Informational;
        request
            .metadata
            .insert("execute_requested".to_string(), Value::Bool(true));
        let mut ctx = AgentContext::default();
        ctx.current_url = Some("https://example.com/current".to_string());
        request = request.with_context(ctx);

        normalize_plan(&mut plan, &request);

        let observed_url = plan.steps.iter().find_map(|step| match &step.tool.kind {
            AgentToolKind::Custom { name, payload }
                if name.eq_ignore_ascii_case("data.extract-site") =>
            {
                payload
                    .as_object()
                    .and_then(|obj| obj.get("url"))
                    .and_then(Value::as_str)
                    .map(|value| value.to_string())
            }
            _ => None,
        });
        assert_eq!(
            observed_url,
            Some("https://example.com/current".to_string())
        );
    }

    #[test]
    fn weather_pipeline_deduplicates_deliver_steps() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "weather dedup");
        plan.push_step(AgentPlanStep::new(
            "obs-weather",
            "观察天气",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.extract-site".to_string(),
                    payload: json!({ "url": "https://www.baidu.com" }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(1_000),
            },
        ));
        for idx in 0..2 {
            plan.push_step(AgentPlanStep::new(
                format!("deliver-weather-{}", idx),
                "交付天气".to_string(),
                AgentTool {
                    kind: AgentToolKind::Custom {
                        name: "data.deliver.structured".to_string(),
                        payload: json!({
                            "schema": "weather_report_v1",
                            "artifact_label": "structured.weather_report_v1",
                            "filename": "weather_report_v1.json",
                            "source_step_id": "obs-weather",
                        }),
                    },
                    wait: WaitMode::None,
                    timeout_ms: Some(1_000),
                },
            ));
        }

        let request = AgentRequest::new(task_id, "查询北京天气");
        normalize_plan(&mut plan, &request);

        let weather_delivers = plan
            .steps
            .iter()
            .filter(|step| match &step.tool.kind {
                AgentToolKind::Custom { name, payload }
                    if name.eq_ignore_ascii_case("data.deliver.structured") =>
                {
                    payload
                        .as_object()
                        .and_then(|obj| obj.get("schema"))
                        .and_then(Value::as_str)
                        .map(|value| value.contains("weather_report_v1"))
                        .unwrap_or(false)
                }
                _ => false,
            })
            .count();
        assert_eq!(weather_delivers, 1, "should keep only one weather deliver");
    }

    #[test]
    fn observation_retarged_to_search_results() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "search-observe");
        plan.push_step(AgentPlanStep::new(
            "llm-step-4",
            "观察搜索结果",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.extract-site".to_string(),
                    payload: json!({
                        "url": "https://www.baidu.com",
                        "title": "观察搜索结果",
                        "detail": "",
                    }),
                },
                wait: WaitMode::Idle,
                timeout_ms: Some(5_000),
            },
        ));
        let request = AgentRequest::new(task_id, "查询今天天气");
        normalize_plan(&mut plan, &request);

        let adjusted_url = plan.steps.iter().find_map(|step| match &step.tool.kind {
            AgentToolKind::Custom { name, payload }
                if name.eq_ignore_ascii_case("data.extract-site") =>
            {
                payload
                    .as_object()
                    .and_then(|obj| obj.get("url"))
                    .and_then(Value::as_str)
                    .map(|url| url.to_string())
            }
            _ => None,
        });
        assert!(matches!(adjusted_url, Some(url) if url.contains("baidu.com/s?")));
    }
}

#[cfg(test)]
mod observe_stage_tests {
    use super::*;

    #[test]
    fn inserts_observation_after_act_steps() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "weather-search");
        plan.steps.push(AgentPlanStep {
            id: "llm-step-1".into(),
            title: "导航到百度".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Navigate {
                    url: "https://www.baidu.com".into(),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        });
        plan.steps.push(AgentPlanStep {
            id: "llm-step-2".into(),
            title: "输入关键词".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::TypeText {
                    locator: AgentLocator::Css("input#kw".into()),
                    text: "济南天气".into(),
                    submit: false,
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(8_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        });
        plan.steps.push(AgentPlanStep {
            id: "llm-step-3".into(),
            title: "提交搜索".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Click {
                    locator: AgentLocator::Css("input#su".into()),
                },
                wait: WaitMode::Idle,
                timeout_ms: Some(8_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        });

        let mut request = AgentRequest::new(task_id, "查询济南天气");
        request.intent.intent_kind = AgentIntentKind::Informational;

        normalize_plan(&mut plan, &request);

        let observe_idx = plan
            .steps
            .iter()
            .position(|step| {
                matches!(
                    &step.tool.kind,
                    AgentToolKind::Custom { name, .. } if name == "data.extract-site"
                )
            })
            .expect("observation step present");
        let mut act_indices = Vec::new();
        for (idx, step) in plan.steps.iter().enumerate() {
            let is_type = matches!(step.tool.kind, AgentToolKind::TypeText { .. });
            let is_click = matches!(step.tool.kind, AgentToolKind::Click { .. });
            let is_search = matches!(
                &step.tool.kind,
                AgentToolKind::Custom { ref name, .. }
                    if name.eq_ignore_ascii_case("browser.search")
                        || name.eq_ignore_ascii_case("browser.search.click-result")
                        || name.eq_ignore_ascii_case("weather.search")
            );
            if is_type || is_click || is_search {
                act_indices.push(idx);
            }
        }
        assert!(
            !act_indices.is_empty(),
            "act stage must contain typing/click/search steps"
        );
        if let Some(last_act) = act_indices.into_iter().max() {
            assert!(
                observe_idx > last_act,
                "observation must occur after the final act step"
            );
        }
    }

    #[test]
    fn url_wait_condition_handles_queries() {
        let expected = "https://www.baidu.com/s?wd=%E6%B5%8E%E5%8D%97%E5%A4%A9%E6%B0%94";
        match build_url_wait_condition(expected) {
            AgentWaitCondition::UrlMatches(pattern) => {
                assert!(pattern.contains("wd=%E6%B5%8E%E5%8D%97%E5%A4%A9%E6%B0%94"))
            }
            other => panic!("unexpected wait condition: {other:?}"),
        }

        let literal = "https://example.com/weather";
        match build_url_wait_condition(literal) {
            AgentWaitCondition::UrlEquals(actual) => assert_eq!(actual, literal),
            other => panic!("literal expectation returned {other:?}"),
        }
    }

    #[test]
    fn weather_macro_prunes_followup_act_steps() {
        let task_id = TaskId::new();
        let mut plan = AgentPlan::new(task_id.clone(), "weather");
        plan.steps.push(AgentPlanStep {
            id: "macro".into(),
            title: "天气搜索".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "weather.search".to_string(),
                    payload: json!({ "query": "济南天气" }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(30_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        });
        plan.steps.push(AgentPlanStep {
            id: "type".into(),
            title: "输入".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::TypeText {
                    locator: AgentLocator::Css("input#kw".into()),
                    text: "济南天气".into(),
                    submit: false,
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(8_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        });
        plan.steps.push(AgentPlanStep {
            id: "click".into(),
            title: "点击".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Click {
                    locator: AgentLocator::Css("#su".into()),
                },
                wait: WaitMode::Idle,
                timeout_ms: Some(8_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        });
        plan.steps.push(AgentPlanStep {
            id: "observe".into(),
            title: "观察".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.extract-site".to_string(),
                    payload: json!({}),
                },
                wait: WaitMode::Idle,
                timeout_ms: Some(5_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        });
        plan.steps.push(AgentPlanStep {
            id: "parse".into(),
            title: "解析".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.parse.weather".to_string(),
                    payload: json!({}),
                },
                wait: WaitMode::None,
                timeout_ms: Some(5_000),
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: HashMap::new(),
        });

        let request = AgentRequest::new(TaskId::new(), "查询济南天气");
        normalize_plan(&mut plan, &request);
        assert!(plan.steps.iter().all(|step| {
            !matches!(
                step.tool.kind,
                AgentToolKind::TypeText { .. } | AgentToolKind::Click { .. }
            )
        }));
        assert!(plan.steps.iter().any(|step| matches!(
            step.tool.kind,
            AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case("weather.search")
        )));
        assert!(plan.steps.iter().any(|step| matches!(
            step.tool.kind,
            AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case("data.parse.weather")
        )));
    }
}

fn unique_step_id(plan: &AgentPlan, base: &str) -> String {
    if plan.steps.iter().all(|step| step.id != base) {
        return base.to_string();
    }
    let mut counter = 1;
    loop {
        let candidate = format!("{}-{}", base, counter);
        if plan.steps.iter().all(|step| step.id != candidate) {
            return candidate;
        }
        counter += 1;
    }
}

fn infer_schema_from_previous_parse(plan: &AgentPlan, deliver_index: usize) -> Option<String> {
    plan.steps
        .iter()
        .take(deliver_index)
        .rev()
        .find_map(|step| match &step.tool.kind {
            AgentToolKind::Custom { name, payload } => schema_for_parse_tool(name, payload),
            _ => None,
        })
}

fn schema_for_parse_tool(name: &str, payload: &Value) -> Option<String> {
    let key = name.trim().to_ascii_lowercase();
    match key.as_str() {
        "data.parse.generic" => payload
            .as_object()
            .and_then(|obj| obj.get("schema"))
            .and_then(Value::as_str)
            .map(|raw| raw.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| Some("generic_observation_v1".to_string())),
        "data.parse.market_info" => Some("market_info_v1".to_string()),
        "data.parse.news_brief" => Some("news_brief_v1".to_string()),
        "data.parse.weather" => Some("weather_report_v1".to_string()),
        "data.parse.github-repo" | "github.extract-repo" | "data.parse.github.extract-repo" => {
            Some("github_repos_v1".to_string())
        }
        "data.parse.twitter-feed" => Some("twitter_feed_v1".to_string()),
        "data.parse.facebook-feed" => Some("facebook_feed_v1".to_string()),
        "data.parse.linkedin-profile" => Some("linkedin_profile_v1".to_string()),
        "data.parse.hackernews-feed" => Some("hackernews_feed_v1".to_string()),
        _ => None,
    }
}

fn payload_has_username(payload: &Value) -> bool {
    payload
        .as_object()
        .and_then(|obj| obj.get("username"))
        .and_then(Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn infer_github_username_for_step(
    plan: &AgentPlan,
    idx: usize,
    request: &AgentRequest,
) -> Option<String> {
    if idx > 0 {
        if let Some(handle) = plan.steps[..idx]
            .iter()
            .rev()
            .find_map(github_username_from_step)
        {
            return Some(handle);
        }
    }

    if let Some(handle) = plan
        .steps
        .iter()
        .skip(idx + 1)
        .find_map(github_username_from_step)
    {
        return Some(handle);
    }

    request
        .context
        .as_ref()
        .and_then(|ctx| ctx.current_url.as_deref())
        .and_then(github_username_from_url)
}

fn github_username_from_step(step: &AgentPlanStep) -> Option<String> {
    match &step.tool.kind {
        AgentToolKind::Navigate { url } => github_username_from_url(url),
        AgentToolKind::Custom { name, payload }
            if name.eq_ignore_ascii_case("data.parse.github-repo") =>
        {
            payload_username(payload)
        }
        _ => None,
    }
}

fn github_username_from_url(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw).ok()?;
    if !parsed
        .host_str()
        .map(|host| host.eq_ignore_ascii_case("github.com"))
        .unwrap_or(false)
    {
        return None;
    }

    let mut segments = parsed
        .path_segments()
        .map(|segments| segments.filter(|segment| !segment.is_empty()))?;

    let first = segments.next()?;
    if first.eq_ignore_ascii_case("orgs") || first.eq_ignore_ascii_case("users") {
        let candidate = segments.next()?;
        if segments.next().is_none() {
            return Some(candidate.to_string());
        }
        return None;
    }

    if segments.next().is_none() {
        return Some(first.to_string());
    }

    None
}

fn payload_username(payload: &Value) -> Option<String> {
    let raw = payload
        .as_object()
        .and_then(|obj| obj.get("username"))
        .and_then(Value::as_str)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn ensure_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("value just set to object")
}

fn normalize_step_tool(step: &mut AgentPlanStep) -> bool {
    let AgentToolKind::Custom { name, payload } = &mut step.tool.kind else {
        return false;
    };

    if let Some(new_kind) = browser_tool_from_alias(name, payload) {
        step.tool.kind = new_kind;
        return true;
    }

    if let Some(canonical) = canonical_tool_name(name) {
        if canonical != name {
            *name = canonical.to_string();
            return true;
        }
    }

    false
}

fn canonical_tool_name(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        return None;
    }
    let lowered = name.trim().to_ascii_lowercase();
    if let Some(canonical) = plugin_custom_alias(&lowered) {
        return Some(canonical);
    }
    let canonical = match lowered.as_str() {
        // Observation aliases
        "observe" | "page.observe" | "page.capture" | "data.observe" => OBSERVATION_CANONICAL,
        // Parse aliases
        "parse" => GENERIC_PARSE_CANONICAL,
        "github.extract-repo" | "data.parse.github.extract-repo" => "data.parse.github-repo",
        "data.parse.twitter_feed" | "data.parse.twitter.feed" => "data.parse.twitter-feed",
        "data.parse.facebook_feed" | "data.parse.facebook.feed" => "data.parse.facebook-feed",
        "data.parse.linkedin_profile" | "data.parse.linkedin.profile" => {
            "data.parse.linkedin-profile"
        }
        "data.parse.hackernews_feed" | "data.parse.hackernews.feed" => "data.parse.hackernews-feed",
        "data.parse.news-brief" => "data.parse.news_brief",
        "data.parse.market-info" => "data.parse.market_info",
        // Deliver aliases
        "deliver"
        | "deliver.structured"
        | "deliver_structured"
        | "data.deliver_structured"
        | "data.deliver-structured"
        | "data.deliver.json" => DELIVER_CANONICAL,
        _ => return None,
    };
    Some(canonical)
}

fn plugin_custom_alias(name: &str) -> Option<&'static str> {
    PLUGIN_CUSTOM_ALIAS_CASES
        .iter()
        .find_map(|(alias, canonical)| (*alias == name).then_some(*canonical))
}

fn browser_tool_from_alias(name: &str, payload: &Value) -> Option<AgentToolKind> {
    let lowered = name.trim().to_ascii_lowercase();
    match lowered.as_str() {
        "browser.navigate" | "browser.goto" | "browser.open" => {
            let url = payload.get("url").and_then(Value::as_str)?.trim();
            if url.is_empty() {
                warn!("browser.navigate missing url payload");
                return None;
            }
            Some(AgentToolKind::Navigate {
                url: url.to_string(),
            })
        }
        "browser.click" => {
            let locator = locator_from_payload(payload)?;
            Some(AgentToolKind::Click { locator })
        }
        "browser.type" | "browser.fill" | "browser.type_text" | "browser.input" => {
            let locator = locator_from_payload(payload)?;
            let text = payload.get("text").and_then(Value::as_str)?.to_string();
            let submit = payload
                .get("submit")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Some(AgentToolKind::TypeText {
                locator,
                text,
                submit,
            })
        }
        "browser.select" => {
            let locator = locator_from_payload(payload)?;
            let value = payload.get("value").and_then(Value::as_str)?.to_string();
            let method = payload
                .get("method")
                .and_then(Value::as_str)
                .map(|v| v.to_string());
            Some(AgentToolKind::Select {
                locator,
                value,
                method,
            })
        }
        "browser.scroll" => {
            let target = scroll_target_from_payload(payload)?;
            Some(AgentToolKind::Scroll { target })
        }
        "plugin.auto-scroll" => {
            let target = scroll_target_from_payload(payload).unwrap_or(AgentScrollTarget::Bottom);
            Some(AgentToolKind::Scroll { target })
        }
        "browser.wait" => {
            let condition = wait_condition_from_payload(payload)?;
            Some(AgentToolKind::Wait { condition })
        }
        "browser.extract" | "browser.observe" => Some(AgentToolKind::Custom {
            name: OBSERVATION_CANONICAL.to_string(),
            payload: payload.clone(),
        }),
        _ => None,
    }
}

fn locator_from_payload(payload: &Value) -> Option<AgentLocator> {
    let locator_value = payload.get("locator").or_else(|| payload.get("selector"))?;
    locator_from_value(locator_value)
}

fn locator_from_value(locator_value: &Value) -> Option<AgentLocator> {
    match locator_value {
        Value::String(raw) => locator_from_str(raw),
        Value::Object(map) => {
            if let Some(css) = map.get("css").and_then(Value::as_str) {
                return Some(AgentLocator::Css(css.to_string()));
            }
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                return Some(AgentLocator::Text {
                    content: text.to_string(),
                    exact: map.get("exact").and_then(Value::as_bool).unwrap_or(false),
                });
            }
            if let (Some(role), Some(name)) = (
                map.get("role").and_then(Value::as_str),
                map.get("name").and_then(Value::as_str),
            ) {
                return Some(AgentLocator::Aria {
                    role: role.to_string(),
                    name: name.to_string(),
                });
            }
            None
        }
        _ => None,
    }
}

fn locator_from_str(raw: &str) -> Option<AgentLocator> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("css=") {
        return Some(AgentLocator::Css(rest.trim().to_string()));
    }
    if let Some(rest) = trimmed.strip_prefix("text=") {
        return Some(AgentLocator::Text {
            content: rest.trim().to_string(),
            exact: false,
        });
    }
    if let Some(rest) = trimmed.strip_prefix("aria:") {
        let mut parts = rest.splitn(2, '=');
        let role = parts
            .next()
            .map(|v| v.trim().to_string())
            .unwrap_or_else(|| "button".to_string());
        let name = parts
            .next()
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
        return Some(AgentLocator::Aria { role, name });
    }
    Some(AgentLocator::Css(trimmed.to_string()))
}

fn scroll_target_from_payload(payload: &Value) -> Option<AgentScrollTarget> {
    match payload.get("target")? {
        Value::String(value) => scroll_target_from_str(value),
        Value::Object(map) => {
            if let Some(kind) = map.get("kind").and_then(Value::as_str) {
                match kind {
                    "top" => return Some(AgentScrollTarget::Top),
                    "bottom" => return Some(AgentScrollTarget::Bottom),
                    "pixels" => {
                        if let Some(amount) = map.get("value").and_then(Value::as_i64) {
                            return Some(AgentScrollTarget::Pixels(amount as i32));
                        }
                    }
                    "element" => {
                        if let Some(anchor) = map.get("anchor") {
                            let locator = locator_from_value(anchor)?;
                            return Some(AgentScrollTarget::Selector(locator));
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        _ => None,
    }
}

fn scroll_target_from_str(value: &str) -> Option<AgentScrollTarget> {
    let trimmed = value.trim();
    let lowered = trimmed.to_ascii_lowercase();
    if lowered == "top" {
        return Some(AgentScrollTarget::Top);
    }
    if lowered == "bottom" {
        return Some(AgentScrollTarget::Bottom);
    }
    if let Some(rest) = lowered.strip_prefix("pixels=") {
        if let Ok(amount) = rest.trim().parse::<i32>() {
            return Some(AgentScrollTarget::Pixels(amount));
        }
    }
    locator_from_str(trimmed).map(AgentScrollTarget::Selector)
}

fn wait_condition_from_payload(payload: &Value) -> Option<AgentWaitCondition> {
    if let Some(duration) = payload.get("duration_ms").and_then(Value::as_u64) {
        return Some(AgentWaitCondition::Duration(duration));
    }
    if let Some(net_quiet) = payload.get("network_idle_ms").and_then(Value::as_u64) {
        return Some(AgentWaitCondition::NetworkIdle(net_quiet));
    }
    if let Some(locator) = locator_from_payload(payload) {
        let state = payload
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("visible")
            .to_ascii_lowercase();
        return match state.as_str() {
            "hidden" => Some(AgentWaitCondition::ElementHidden(locator)),
            _ => Some(AgentWaitCondition::ElementVisible(locator)),
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{
        AgentContext, AgentLocator, AgentScrollTarget, AgentTool, AgentToolKind, AgentWaitCondition,
    };
    use serde_json::{json, Value};

    #[test]
    fn normalizes_custom_tool_aliases() {
        let mut plan = AgentPlan::new(TaskId::new(), "demo");
        plan.push_step(AgentPlanStep {
            id: "step-1".into(),
            title: "Parse github".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "github.extract-repo".into(),
                    payload: json!({ "username": "demo" }),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: Default::default(),
        });
        plan.push_step(AgentPlanStep {
            id: "step-2".into(),
            title: "Deliver".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.deliver.json".into(),
                    payload: json!({ "schema": "github_repos_v1" }),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: Default::default(),
        });

        let rewrites = normalize_custom_tools(&mut plan);
        assert_eq!(rewrites, 2);
        match &plan.steps[0].tool.kind {
            AgentToolKind::Custom { name, .. } => {
                assert_eq!(name, "data.parse.github-repo");
            }
            _ => panic!("expected custom tool"),
        }
        match &plan.steps[1].tool.kind {
            AgentToolKind::Custom { name, .. } => {
                assert_eq!(name, DELIVER_CANONICAL);
            }
            _ => panic!("expected custom tool"),
        }
    }

    #[test]
    fn normalizes_plain_deliver_alias() {
        let mut plan = AgentPlan::new(TaskId::new(), "deliver alias");
        plan.push_step(AgentPlanStep {
            id: "deliver".into(),
            title: "交付结构化数据".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "deliver".into(),
                    payload: json!({ "schema": "github_repos_v1" }),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: Default::default(),
        });

        let rewrites = normalize_custom_tools(&mut plan);
        assert_eq!(rewrites, 1);
        match &plan.steps[0].tool.kind {
            AgentToolKind::Custom { name, .. } => {
                assert_eq!(name, DELIVER_CANONICAL);
            }
            _ => panic!("expected custom tool"),
        }
    }

    #[test]
    fn converts_browser_aliases_into_standard_tools() {
        let mut plan = AgentPlan::new(TaskId::new(), "browser aliases");
        plan.push_step(AgentPlanStep {
            id: "nav".into(),
            title: "Navigate".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "browser.navigate".into(),
                    payload: json!({ "url": "https://example.com" }),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: Default::default(),
        });
        plan.push_step(AgentPlanStep {
            id: "click".into(),
            title: "Click".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "browser.click".into(),
                    payload: json!({ "locator": "css=.cta" }),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: Default::default(),
        });
        plan.push_step(AgentPlanStep {
            id: "type".into(),
            title: "Type".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "browser.type".into(),
                    payload: json!({ "locator": "text=Search", "text": "rustaceans", "submit": true }),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: Default::default(),
        });
        plan.push_step(AgentPlanStep {
            id: "scroll".into(),
            title: "Scroll".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "browser.scroll".into(),
                    payload: json!({ "target": "bottom" }),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: Default::default(),
        });
        plan.push_step(AgentPlanStep {
            id: "wait".into(),
            title: "Wait".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "browser.wait".into(),
                    payload: json!({ "locator": "css=.ready", "state": "visible" }),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: Default::default(),
        });

        let rewrites = normalize_custom_tools(&mut plan);
        assert_eq!(rewrites, 5);

        assert!(matches!(
            plan.steps[0].tool.kind,
            AgentToolKind::Navigate { .. }
        ));
        assert!(matches!(
            plan.steps[1].tool.kind,
            AgentToolKind::Click { .. }
        ));
        if let AgentToolKind::TypeText { submit, .. } = &plan.steps[2].tool.kind {
            assert!(submit);
        } else {
            panic!("expected type text");
        }
        assert!(matches!(
            plan.steps[3].tool.kind,
            AgentToolKind::Scroll { .. }
        ));
        assert!(matches!(
            plan.steps[4].tool.kind,
            AgentToolKind::Wait { .. }
        ));
    }

    #[test]
    fn plugin_custom_aliases_map_to_supported_tools() {
        for (alias, canonical) in PLUGIN_CUSTOM_ALIAS_CASES {
            assert_eq!(canonical_tool_name(alias), Some(*canonical));
        }
    }

    #[test]
    fn plugin_auto_scroll_alias_converts_to_scroll_action() {
        let mut plan = AgentPlan::new(TaskId::new(), "plugin scroll");
        plan.push_step(AgentPlanStep {
            id: "scroll".into(),
            title: "Auto scroll".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "plugin.auto-scroll".into(),
                    payload: json!({}),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: Default::default(),
        });

        let rewrites = normalize_custom_tools(&mut plan);
        assert_eq!(rewrites, 1);
        match &plan.steps[0].tool.kind {
            AgentToolKind::Scroll { target } => {
                assert!(matches!(target, AgentScrollTarget::Bottom));
            }
            _ => panic!("expected scroll action"),
        }
    }

    #[test]
    fn retargets_blocked_google_search_to_fallback_engine() {
        let mut plan = AgentPlan::new(TaskId::new(), "search fallback");
        plan.push_step(AgentPlanStep::new(
            "llm-step-1",
            "Google search",
            AgentTool {
                kind: AgentToolKind::Navigate {
                    url: "https://www.google.com/search?q=白银".into(),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
        ));
        plan.push_step(AgentPlanStep::new(
            "llm-step-2",
            "Wait for google",
            AgentTool {
                kind: AgentToolKind::Wait {
                    condition: AgentWaitCondition::UrlEquals(
                        "https://www.google.com/search?q=白银".into(),
                    ),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
        ));

        let mut request = AgentRequest::new(TaskId::new(), "查询白银价格");
        request.metadata.insert(
            "search_base_url".to_string(),
            Value::String("https://www.baidu.com/s?wd=".to_string()),
        );
        normalize_plan(&mut plan, &request);

        let navigate_step = plan
            .steps
            .iter()
            .find(|step| step.id == "llm-step-1")
            .expect("navigate step missing");
        let fallback_url = match &navigate_step.tool.kind {
            AgentToolKind::Navigate { url } => url.clone(),
            _ => panic!("expected navigate tool"),
        };
        assert!(fallback_url.contains("baidu.com"));

        let wait_step = plan
            .steps
            .iter()
            .find(|step| step.id == "llm-step-2")
            .expect("wait step missing");
        match &wait_step.tool.kind {
            AgentToolKind::Wait { condition } => match condition {
                AgentWaitCondition::UrlMatches(pattern) => {
                    assert!(pattern.contains("baidu"));
                }
                AgentWaitCondition::UrlEquals(url) => {
                    assert!(url.contains("baidu"));
                }
                other => panic!("unexpected wait condition: {:?}", other),
            },
            _ => panic!("expected wait step"),
        }
    }

    #[test]
    fn retarget_wait_tools_updates_custom_payload() {
        let mut plan = AgentPlan::new(TaskId::new(), "custom wait fallback");
        plan.push_step(AgentPlanStep::new(
            "llm-step-1",
            "Google search",
            AgentTool {
                kind: AgentToolKind::Navigate {
                    url: "https://www.google.com/search?q=白银".into(),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
        ));
        plan.push_step(AgentPlanStep::new(
            "llm-step-2",
            "wait via custom tool",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "wait-for-condition".into(),
                    payload: json!({
                        "expect": {
                            "url_equals": "https://www.google.com/search?q=白银"
                        }
                    }),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
        ));

        let mut ledger = PlanRepairLedger::new(PLAN_REPAIR_NOTE_BUDGET);
        let context = StageContext {
            current_url: None,
            snapshot_url: None,
            preferred_sites: Vec::new(),
            tenant_default_url: None,
            search_terms: vec!["查询白银价格".to_string()],
            guardrail_keywords: Vec::new(),
            guardrail_keyword_count: 0,
            guardrail_domains: Vec::new(),
            requested_outputs: Vec::new(),
            browser_context: None,
            search_fallback_url: "https://www.baidu.com/s?wd=查询白银价格".to_string(),
            force_observe_current: false,
            auto_act_retry: 0,
            auto_act: AutoActTuning::default(),
        };

        retarget_wait_tools(&mut plan, &context, &mut ledger);

        let wait_step = plan
            .steps
            .iter()
            .find(|step| step.id == "llm-step-2")
            .expect("wait step missing");
        match &wait_step.tool.kind {
            AgentToolKind::Custom { payload, .. } => {
                let payload_str = payload.to_string();
                assert!(payload_str.contains("baidu"), "payload={}", payload_str);
            }
            other => panic!("unexpected tool: {:?}", other),
        }
    }

    #[test]
    fn fills_github_username_from_navigation() {
        let mut plan = AgentPlan::new(TaskId::new(), "github");
        plan.push_step(AgentPlanStep {
            id: "nav".into(),
            title: "Go to profile".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Navigate {
                    url: "https://github.com/example".into(),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: Default::default(),
        });
        plan.push_step(AgentPlanStep {
            id: "parse".into(),
            title: "Parse repos".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.parse.github-repo".into(),
                    payload: json!({}),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: Default::default(),
        });
        let request = AgentRequest::new(TaskId::new(), "goal");

        let mut ledger = PlanRepairLedger::new(PLAN_REPAIR_NOTE_BUDGET);
        let rewrites = ensure_github_repo_usernames(&mut plan, &request, &mut ledger);
        assert_eq!(rewrites, 1);

        match &plan.steps[1].tool.kind {
            AgentToolKind::Custom { payload, .. } => {
                let username = payload
                    .as_object()
                    .and_then(|obj| obj.get("username"))
                    .and_then(Value::as_str)
                    .unwrap();
                assert_eq!(username, "example");
            }
            _ => panic!("expected custom tool"),
        }
    }

    #[test]
    fn fills_github_username_from_context_when_missing_navigation() {
        let mut plan = AgentPlan::new(TaskId::new(), "github context");
        plan.push_step(AgentPlanStep {
            id: "parse".into(),
            title: "Parse repos".into(),
            detail: String::new(),
            tool: AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.parse.github-repo".into(),
                    payload: json!({}),
                },
                wait: WaitMode::DomReady,
                timeout_ms: None,
            },
            validations: Vec::new(),
            requires_approval: false,
            metadata: Default::default(),
        });
        let mut request = AgentRequest::new(TaskId::new(), "goal");
        request.context = Some(AgentContext {
            current_url: Some("https://github.com/sample".into()),
            ..Default::default()
        });

        let mut ledger = PlanRepairLedger::new(PLAN_REPAIR_NOTE_BUDGET);
        let rewrites = ensure_github_repo_usernames(&mut plan, &request, &mut ledger);
        assert_eq!(rewrites, 1);

        match &plan.steps[0].tool.kind {
            AgentToolKind::Custom { payload, .. } => {
                let username = payload
                    .as_object()
                    .and_then(|obj| obj.get("username"))
                    .and_then(Value::as_str)
                    .unwrap();
                assert_eq!(username, "sample");
            }
            _ => panic!("expected custom tool"),
        }
    }

    #[test]
    fn enforces_domready_wait_for_typing_after_navigation() {
        let mut plan = AgentPlan::new(TaskId::new(), "type after nav");
        plan.push_step(AgentPlanStep::new(
            "nav",
            "Navigate",
            AgentTool {
                kind: AgentToolKind::Navigate {
                    url: "https://example.com".into(),
                },
                wait: WaitMode::Idle,
                timeout_ms: Some(5_000),
            },
        ));
        plan.push_step(AgentPlanStep::new(
            "type",
            "Type",
            AgentTool {
                kind: AgentToolKind::TypeText {
                    locator: AgentLocator::Css("input[name=q]".into()),
                    text: "query".into(),
                    submit: true,
                },
                wait: WaitMode::None,
                timeout_ms: None,
            },
        ));

        apply_execution_tweaks(&mut plan);

        assert!(matches!(plan.steps[1].tool.wait, WaitMode::DomReady));
    }

    #[test]
    fn leaves_typing_wait_unchanged_without_navigation() {
        let mut plan = AgentPlan::new(TaskId::new(), "type standalone");
        plan.push_step(AgentPlanStep::new(
            "type",
            "Type",
            AgentTool {
                kind: AgentToolKind::TypeText {
                    locator: AgentLocator::Css("input[name=q]".into()),
                    text: "query".into(),
                    submit: false,
                },
                wait: WaitMode::None,
                timeout_ms: None,
            },
        ));

        apply_execution_tweaks(&mut plan);

        assert!(matches!(plan.steps[0].tool.wait, WaitMode::None));
    }

    #[test]
    fn stage_auditor_inserts_search_step_for_informational_requests() {
        let mut plan = AgentPlan::new(TaskId::new(), "informational");
        plan.push_step(AgentPlanStep::new(
            "nav",
            "导航至搜索结果",
            AgentTool {
                kind: AgentToolKind::Navigate {
                    url: "https://www.baidu.com/s?wd=%E6%9C%80%E9%AB%98%E4%BA%BA%E6%B0%91%E6%A3%80%E5%AF%9F%E9%99%A2".to_string(),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
        ));

        let mut request = AgentRequest::new(TaskId::new(), "我想看下现在办理最多的案件是那种");
        request.intent.intent_kind = AgentIntentKind::Informational;
        request.intent.target_sites = vec!["https://stats.gov.cn".to_string()];
        let context = ContextResolver::new(&request).build();

        let mut ledger = PlanRepairLedger::new(8);
        StageAuditor::new(&mut plan, &request, context, &mut ledger, true).audit();

        assert!(
            plan.steps.iter().any(|step| match &step.tool.kind {
                AgentToolKind::Custom { name, .. } => name.eq_ignore_ascii_case("browser.search"),
                _ => false,
            }),
            "Plan tools: {:?}",
            plan.steps
                .iter()
                .map(|step| format!("{:?}", step.tool.kind))
                .collect::<Vec<_>>()
        );
        assert!(
            plan.steps
                .iter()
                .any(|step| matches!(step.tool.kind, AgentToolKind::Click { .. })),
            "Plan tools: {:?}",
            plan.steps
                .iter()
                .map(|step| format!("{:?}", step.tool.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn informational_pipeline_includes_observe_validate_and_deliver() {
        let mut plan = AgentPlan::new(TaskId::new(), "informational-observe");
        plan.push_step(AgentPlanStep::new(
            "nav",
            "打开首页",
            AgentTool {
                kind: AgentToolKind::Navigate {
                    url: "https://www.baidu.com".to_string(),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(30_000),
            },
        ));

        use crate::agent::guardrails::{derive_guardrail_domains, derive_guardrail_keywords};

        let mut request = AgentRequest::new(TaskId::new(), "我想看下现在办理最多的案件是那种");
        request.intent.intent_kind = AgentIntentKind::Informational;
        request.intent.validation_keywords = vec!["最高人民检察院 案件".to_string()];
        request.intent.target_sites = vec!["https://stats.gov.cn".to_string()];
        assert!(!derive_guardrail_keywords(&request).is_empty());
        assert!(!derive_guardrail_domains(&request).is_empty());
        let context = ContextResolver::new(&request).build();

        let mut ledger = PlanRepairLedger::new(16);
        StageAuditor::new(&mut plan, &request, context, &mut ledger, true).audit();

        let extract_index = plan
            .steps
            .iter()
            .position(|step| match &step.tool.kind {
                AgentToolKind::Custom { name, .. } => {
                    name.eq_ignore_ascii_case("data.extract-site")
                }
                _ => false,
            })
            .expect("data.extract-site inserted");
        let validate_index = plan
            .steps
            .iter()
            .position(|step| match &step.tool.kind {
                AgentToolKind::Custom { name, .. } => {
                    name.eq_ignore_ascii_case("data.validate-target")
                }
                _ => false,
            })
            .expect("data.validate-target inserted");
        let parse_index = plan
            .steps
            .iter()
            .position(|step| match &step.tool.kind {
                AgentToolKind::Custom { name, .. } => name.starts_with("data.parse"),
                _ => false,
            })
            .expect("data.parse.* inserted");
        let deliver_index = plan
            .steps
            .iter()
            .position(|step| match &step.tool.kind {
                AgentToolKind::Custom { name, .. } => {
                    name.eq_ignore_ascii_case("data.deliver.structured")
                }
                _ => false,
            })
            .expect("data.deliver.structured inserted");

        assert!(extract_index < validate_index);
        assert!(validate_index < parse_index);
        assert!(parse_index < deliver_index);

        let timeline = plan
            .meta
            .vendor_context
            .get("stage_timeline")
            .expect("stage timeline");
        assert!(
            timeline
                .get("stages")
                .and_then(|value| value.as_array())
                .map(|items| items.len())
                .unwrap_or(0)
                >= 4
        );
    }
}

/// Composite result returned to the CLI command.
#[derive(Debug)]
pub struct ChatSessionOutput {
    pub plan: AgentPlan,
    pub explanations: Vec<String>,
    pub flow: PlanToFlowResult,
}

impl ChatSessionOutput {
    pub fn summarize_steps(&self) -> Vec<String> {
        self.plan
            .steps
            .iter()
            .enumerate()
            .map(|(idx, step)| format!("{}. {}", idx + 1, StepSummary(step)))
            .collect()
    }
}

struct StepSummary<'a>(&'a AgentPlanStep);

impl<'a> fmt::Display for StepSummary<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let step = self.0;
        let action = match &step.tool.kind {
            AgentToolKind::Navigate { url } => format!("Navigate to {}", url),
            AgentToolKind::Click { locator } => format!("Click {}", describe_locator(locator)),
            AgentToolKind::TypeText {
                locator,
                text,
                submit,
            } => {
                let submit_note = if *submit { " and submit" } else { "" };
                format!(
                    "Type '{}' into {}{}",
                    text,
                    describe_locator(locator),
                    submit_note
                )
            }
            AgentToolKind::Select {
                locator,
                value,
                method,
            } => {
                let method_note = method.as_deref().unwrap_or("value");
                format!(
                    "Select '{}' by {} via {}",
                    value,
                    method_note,
                    describe_locator(locator)
                )
            }
            AgentToolKind::Scroll { target } => {
                format!("Scroll {}", describe_scroll_target(target))
            }
            AgentToolKind::Wait { condition } => {
                format!("Wait until {}", describe_wait_condition(condition))
            }
            AgentToolKind::Custom { name, .. } => format!("Invoke custom tool '{}'", name),
            AgentToolKind::Done { success, text } => {
                if *success {
                    format!("Complete task: {}", text)
                } else {
                    format!("Abort task: {}", text)
                }
            }
        };

        let wait_note = match step.tool.wait {
            WaitMode::None => String::new(),
            WaitMode::DomReady => String::new(),
            WaitMode::Idle => " (wait for page idle)".to_string(),
        };

        if step.detail.is_empty() {
            write!(f, "{}{}", action, wait_note)
        } else {
            write!(f, "{}{} – {}", action, wait_note, step.detail)
        }
    }
}

fn describe_locator(locator: &AgentLocator) -> String {
    match locator {
        AgentLocator::Css(selector) => format!("CSS selector '{}'", selector),
        AgentLocator::Aria { role, name } => format!("ARIA role '{}' with name '{}'", role, name),
        AgentLocator::Text { content, exact } => {
            if *exact {
                format!("text exactly '{}'", content)
            } else {
                format!("text containing '{}'", content)
            }
        }
    }
}

fn describe_scroll_target(target: &AgentScrollTarget) -> String {
    match target {
        AgentScrollTarget::Top => "to top".to_string(),
        AgentScrollTarget::Bottom => "to bottom".to_string(),
        AgentScrollTarget::Selector(locator) => {
            format!("to {}", describe_locator(locator))
        }
        AgentScrollTarget::Pixels(delta) => {
            if *delta >= 0 {
                format!("by {} pixels down", delta)
            } else {
                format!("by {} pixels up", delta.abs())
            }
        }
    }
}

fn describe_wait_condition(condition: &AgentWaitCondition) -> String {
    match condition {
        AgentWaitCondition::ElementVisible(locator) => {
            format!("{} is visible", describe_locator(locator))
        }
        AgentWaitCondition::ElementHidden(locator) => {
            format!("{} is hidden", describe_locator(locator))
        }
        AgentWaitCondition::UrlMatches(pattern) => {
            format!("URL matches '{}'", pattern)
        }
        AgentWaitCondition::UrlEquals(expected) => {
            format!("URL equals '{}'", expected)
        }
        AgentWaitCondition::TitleMatches(pattern) => {
            format!("title matches '{}'", pattern)
        }
        AgentWaitCondition::NetworkIdle(ms) => {
            format!("network idle for {} ms", ms)
        }
        AgentWaitCondition::Duration(ms) => {
            format!("{} ms elapsed", ms)
        }
    }
}
