pub mod executor;

use agent_core::{
    plan_to_flow, AgentContext, AgentError, AgentLocator, AgentPlan, AgentPlanStep, AgentPlanner,
    AgentRequest, AgentScrollTarget, AgentToolKind, AgentWaitCondition, ConversationRole,
    ConversationTurn, LlmProvider, PlanToFlowOptions, PlanToFlowResult, PlanValidator,
    PlannerConfig, PlannerOutcome, RuleBasedPlanner, WaitMode,
};
use anyhow::{anyhow, Result};
use hex;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use soulbrowser_core_types::TaskId;
use std::{fmt, sync::Arc};
use tracing::{debug, warn};
use url::Url;

use crate::llm::LlmPlanCache;

pub use executor::{execute_plan, FlowExecutionOptions, FlowExecutionReport, StepExecutionStatus};

/// Runner that bridges CLI prompts to either the rule-based or LLM planner.
#[derive(Clone)]
pub struct ChatRunner {
    planner: PlannerStrategy,
    flow_options: PlanToFlowOptions,
}

impl fmt::Debug for ChatRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatRunner")
            .field("planner", &self.planner)
            .finish()
    }
}

#[derive(Clone)]
enum PlannerStrategy {
    Rule(RuleBasedPlanner),
    Llm(LlmPlanner),
}

impl fmt::Debug for PlannerStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlannerStrategy::Rule(_) => f.write_str("PlannerStrategy::Rule"),
            PlannerStrategy::Llm(planner) => f
                .debug_struct("PlannerStrategy::Llm")
                .field("cache", &planner.cache.is_some())
                .finish(),
        }
    }
}

impl PlannerStrategy {
    async fn plan(&self, request: &AgentRequest) -> Result<PlannerOutcome, AgentError> {
        match self {
            PlannerStrategy::Rule(planner) => planner.draft_plan(request),
            PlannerStrategy::Llm(planner) => planner.plan(request).await,
        }
    }

    async fn replan(
        &self,
        request: &AgentRequest,
        previous_plan: &AgentPlan,
        failure_summary: &str,
    ) -> Result<PlannerOutcome, AgentError> {
        match self {
            PlannerStrategy::Rule(planner) => planner.draft_plan(request),
            PlannerStrategy::Llm(planner) => {
                planner
                    .replan(request, previous_plan, failure_summary)
                    .await
            }
        }
    }
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
        Self {
            planner: PlannerStrategy::Rule(RuleBasedPlanner::new(config)),
            flow_options,
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
        self.planner = PlannerStrategy::Llm(LlmPlanner::new(provider, cache));
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
        self.finalize_outcome(outcome, &request)
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
        self.finalize_outcome(outcome, &request)
    }

    fn finalize_outcome(
        &self,
        mut outcome: PlannerOutcome,
        request: &AgentRequest,
    ) -> Result<ChatSessionOutput> {
        let rewrites = normalize_custom_tools(&mut outcome.plan);
        if rewrites > 0 {
            debug!(rewrites, "normalized custom tool aliases in plan");
        }
        let autofills = apply_plan_enhancements(&mut outcome.plan, request);
        if autofills > 0 {
            debug!(autofills, "auto-filled planner payload gaps");
        }
        apply_execution_tweaks(&mut outcome.plan);
        PlanValidator::default()
            .validate(&outcome.plan, request)
            .map_err(|err| anyhow!("plan failed validation: {}", err))?;
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

fn normalize_custom_tools(plan: &mut AgentPlan) -> usize {
    let mut rewrites = 0;
    for step in plan.steps.iter_mut() {
        if normalize_step_tool(step) {
            rewrites += 1;
        }
    }
    rewrites
}

fn apply_plan_enhancements(plan: &mut AgentPlan, request: &AgentRequest) -> usize {
    ensure_github_repo_usernames(plan, request)
}

fn apply_execution_tweaks(plan: &mut AgentPlan) {
    const MIN_NAV_TIMEOUT_MS: u64 = 30_000;
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
            }
            _ => {}
        }
    }
}

fn ensure_github_repo_usernames(plan: &mut AgentPlan, request: &AgentRequest) -> usize {
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
                map.insert("username".to_string(), Value::String(username));
                updates += 1;
            }
        }
    }
    updates
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
        "data.deliver_structured" | "data.deliver-structured" | "data.deliver.json" => {
            DELIVER_CANONICAL
        }
        _ => return None,
    };
    Some(canonical)
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
    use agent_core::{AgentContext, AgentTool, AgentToolKind};
    use serde_json::json;

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

        let rewrites = ensure_github_repo_usernames(&mut plan, &request);
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

        let rewrites = ensure_github_repo_usernames(&mut plan, &request);
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
