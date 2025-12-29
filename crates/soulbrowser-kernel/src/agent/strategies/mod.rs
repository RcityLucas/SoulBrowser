use std::collections::HashMap;
use std::sync::Arc;

use agent_core::{
    plan::{AgentPlan, AgentPlanStep, AgentTool, AgentValidation},
    planner::PlanStageKind,
    AgentRequest,
};
use serde_json::{json, Value};

use crate::agent::stage_context::StageContext;

mod act;
mod deliver;
mod navigate;
mod observe;
mod parse;

pub use act::*;
pub use deliver::*;
pub use navigate::*;
pub use observe::*;
pub use parse::*;

#[derive(Debug)]
pub struct StrategyInput<'a> {
    pub plan: &'a AgentPlan,
    pub request: &'a AgentRequest,
    pub context: &'a StageContext,
}

#[derive(Debug, Clone)]
pub struct StrategyStep {
    pub title: String,
    pub detail: Option<String>,
    pub tool: AgentTool,
    pub validations: Vec<AgentValidation>,
    pub metadata: HashMap<String, Value>,
}

impl StrategyStep {
    pub fn new(title: impl Into<String>, tool: AgentTool) -> Self {
        Self {
            title: title.into(),
            detail: None,
            tool,
            validations: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct StrategyApplication {
    pub steps: Vec<StrategyStep>,
    pub note: Option<String>,
    pub overlay: Option<Value>,
}

pub(crate) fn stage_overlay(
    stage: PlanStageKind,
    strategy: impl Into<String>,
    status: impl Into<String>,
    detail: impl Into<String>,
) -> Value {
    let label = stage_label(stage);
    let detail = detail.into();
    json!({
        "stage": stage.as_str(),
        "strategy": strategy.into(),
        "status": status.into(),
        "title": format!("{}阶段", label),
        "detail": detail,
        "message": detail,
    })
}

pub(crate) fn stage_label(stage: PlanStageKind) -> &'static str {
    match stage {
        PlanStageKind::Navigate => "导航",
        PlanStageKind::Observe => "观察",
        PlanStageKind::Act => "执行",
        PlanStageKind::Parse => "解析",
        PlanStageKind::Deliver => "交付",
    }
}

pub trait StageStrategy: Send + Sync {
    fn id(&self) -> &'static str;
    fn stage(&self) -> PlanStageKind;
    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication>;
}

pub struct StrategyRegistry {
    entries: HashMap<String, Arc<dyn StageStrategy>>,
}

impl StrategyRegistry {
    pub fn builtin() -> Self {
        let mut registry = Self {
            entries: HashMap::new(),
        };
        registry.register(ContextUrlNavigateStrategy::new());
        registry.register(PreferredSiteNavigateStrategy::new());
        registry.register(SearchNavigateStrategy::new());
        registry.register(WeatherSearchStrategy::new());
        registry.register(ExtractSiteObserveStrategy::new());
        registry.register(AutoActStrategy::new());
        registry.register(GenericParseStrategy::new());
        registry.register(WeatherParseStrategy::new());
        registry.register(LlmSummaryStrategy::new());
        registry.register(StructuredDeliverStrategy::new());
        registry.register(AgentNoteStrategy::new());
        registry
    }

    fn register<S: StageStrategy + 'static>(&mut self, strategy: S) {
        self.entries
            .insert(strategy.id().to_string(), Arc::new(strategy));
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn StageStrategy>> {
        self.entries.get(id).cloned()
    }
}

pub fn materialize_step(template: &StrategyStep, id: String) -> AgentPlanStep {
    AgentPlanStep {
        id,
        title: template.title.clone(),
        detail: template.detail.clone().unwrap_or_default(),
        tool: template.tool.clone(),
        validations: template.validations.clone(),
        requires_approval: false,
        metadata: template.metadata.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{
        plan::{AgentLocator, AgentScrollTarget, AgentToolKind},
        AgentPlan, AgentPlanStep, AgentTool, WaitMode,
    };
    use serde_json::json;
    use soulbrowser_core_types::TaskId;

    fn base_request() -> AgentRequest {
        AgentRequest::new(TaskId::new(), "demo goal")
    }

    fn base_context() -> StageContext {
        StageContext {
            current_url: None,
            snapshot_url: None,
            preferred_sites: vec![],
            tenant_default_url: None,
            search_terms: vec!["demo".to_string()],
            requested_outputs: Vec::new(),
            browser_context: None,
            search_fallback_url: "https://www.baidu.com/s?wd=demo".to_string(),
            force_observe_current: false,
        }
    }

    fn populated_context() -> StageContext {
        StageContext {
            current_url: Some("https://example.com".to_string()),
            ..base_context()
        }
    }

    fn baidu_context() -> StageContext {
        StageContext {
            current_url: Some("https://www.baidu.com".to_string()),
            ..base_context()
        }
    }

    #[test]
    fn context_url_strategy_generates_navigate_step() {
        let plan = AgentPlan::new(TaskId::new(), "demo");
        let request = base_request();
        let context = populated_context();
        let strategy = ContextUrlNavigateStrategy::new();
        let input = StrategyInput {
            plan: &plan,
            request: &request,
            context: &context,
        };
        let result = strategy.apply(&input).expect("strategy applied");
        assert_eq!(result.steps.len(), 1);
        match &result.steps[0].tool.kind {
            AgentToolKind::Navigate { url } => {
                assert_eq!(url, "https://example.com");
            }
            _ => panic!("expected navigate tool"),
        }
    }

    #[test]
    fn deliver_strategy_targets_parse_step() {
        let mut plan = AgentPlan::new(TaskId::new(), "demo");
        plan.push_step(AgentPlanStep::new(
            "step-1",
            "Parse",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.parse.generic".to_string(),
                    payload: json!({ "schema": "generic_observation_v1" }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(1_000),
            },
        ));
        let request = base_request();
        let context = base_context();
        let strategy = StructuredDeliverStrategy::new();
        let input = StrategyInput {
            plan: &plan,
            request: &request,
            context: &context,
        };
        let result = strategy.apply(&input).expect("deliver strategy applied");
        assert_eq!(result.steps.len(), 1);
        match &result.steps[0].tool.kind {
            AgentToolKind::Custom { name, payload } => {
                assert_eq!(name, "data.deliver.structured");
                assert_eq!(
                    payload
                        .as_object()
                        .and_then(|obj| obj.get("schema"))
                        .and_then(|value| value.as_str()),
                    Some("generic_observation_v1")
                );
            }
            _ => panic!("expected deliver tool"),
        }
    }

    #[test]
    fn llm_summary_strategy_emits_parse_and_note() {
        let mut plan = AgentPlan::new(TaskId::new(), "demo");
        plan.push_step(AgentPlanStep::new(
            "obs-1",
            "观察",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.extract-site".to_string(),
                    payload: json!({ "url": "https://example.com" }),
                },
                wait: WaitMode::None,
                timeout_ms: Some(1_000),
            },
        ));
        let request = base_request();
        let context = base_context();
        let strategy = LlmSummaryStrategy::new();
        let input = StrategyInput {
            plan: &plan,
            request: &request,
            context: &context,
        };
        let result = strategy.apply(&input).expect("llm summary available");
        assert_eq!(result.steps.len(), 2);
        assert!(matches!(
            &result.steps[0].tool.kind,
            AgentToolKind::Custom { name, .. } if name == "data.parse.generic"
        ));
        assert!(matches!(
            &result.steps[1].tool.kind,
            AgentToolKind::Custom { name, .. } if name == "agent.note"
        ));
    }

    #[test]
    fn auto_act_strategy_scrolls_page() {
        let plan = AgentPlan::new(TaskId::new(), "demo");
        let request = base_request();
        let context = populated_context();
        let strategy = AutoActStrategy::new();
        let input = StrategyInput {
            plan: &plan,
            request: &request,
            context: &context,
        };
        let result = strategy.apply(&input).expect("auto act applied");
        assert_eq!(result.steps.len(), 1);
        match &result.steps[0].tool.kind {
            AgentToolKind::Scroll { target } => match target {
                AgentScrollTarget::Pixels(value) => assert_eq!(*value, 720),
                _ => panic!("expected pixel scroll"),
            },
            other => panic!("unexpected tool: {other:?}"),
        }
    }

    #[test]
    fn auto_act_strategy_types_and_clicks_on_baidu() {
        let plan = AgentPlan::new(TaskId::new(), "demo");
        let request = base_request();
        let context = baidu_context();
        let strategy = AutoActStrategy::new();
        let input = StrategyInput {
            plan: &plan,
            request: &request,
            context: &context,
        };
        let result = strategy.apply(&input).expect("baidu act applied");
        assert_eq!(result.steps.len(), 2);
        assert!(matches!(
            &result.steps[0].tool.kind,
            AgentToolKind::TypeText { locator, submit, .. }
                if matches!(locator, AgentLocator::Css(selector) if selector == "input#kw") && !submit
        ));
        assert!(matches!(
            &result.steps[1].tool.kind,
            AgentToolKind::Click { locator }
                if matches!(locator, AgentLocator::Css(selector) if selector == "input#su")
        ));
    }

    #[test]
    fn context_strategy_uses_fallback_when_missing_url() {
        let plan = AgentPlan::new(TaskId::new(), "demo");
        let request = base_request();
        let context = base_context();
        let strategy = ContextUrlNavigateStrategy::new();
        let input = StrategyInput {
            plan: &plan,
            request: &request,
            context: &context,
        };
        let result = strategy.apply(&input).expect("fallback navigate");
        assert!(result.steps.iter().any(|step| match &step.tool.kind {
            AgentToolKind::Navigate { url } => url.contains("baidu.com"),
            _ => false,
        }));
    }
}
