use agent_core::plan::{
    AgentLocator, AgentPlan, AgentTool, AgentToolKind, AgentValidation, AgentWaitCondition,
};
use agent_core::WaitMode;
use serde_json::json;

use crate::agent::EXPECTED_URL_METADATA_KEY;

use super::{stage_overlay, StageStrategy, StrategyApplication, StrategyInput, StrategyStep};

#[derive(Debug, Default)]
pub struct ExtractSiteObserveStrategy;

impl ExtractSiteObserveStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for ExtractSiteObserveStrategy {
    fn id(&self) -> &'static str {
        "extract_site"
    }

    fn stage(&self) -> agent_core::planner::PlanStageKind {
        agent_core::planner::PlanStageKind::Observe
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        let Some(url) = resolve_target_url(
            input.plan,
            input.context,
            input.context.should_force_current_observation(),
        ) else {
            return None;
        };
        let target_url = url.clone();
        let tool = AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.extract-site".to_string(),
                payload: json!({
                    "url": target_url,
                    "title": "自动采集页面",
                    "detail": "Stage strategy observation",
                }),
            },
            wait: WaitMode::DomReady,
            timeout_ms: Some(10_000),
        };
        let mut step = StrategyStep::new("采集网页内容", tool);
        step.metadata
            .insert(EXPECTED_URL_METADATA_KEY.to_string(), json!(target_url));
        if url.contains("baidu.com/s?") {
            step.validations.push(AgentValidation {
                description: "等待搜索结果加载".to_string(),
                condition: AgentWaitCondition::UrlMatches(url.clone()),
            });
            step.validations.push(AgentValidation {
                description: "等待结果列表出现".to_string(),
                condition: AgentWaitCondition::ElementVisible(AgentLocator::Css(
                    "div#content_left".to_string(),
                )),
            });
        }
        Some(StrategyApplication {
            steps: vec![step],
            note: Some("追加 data.extract-site 采集步骤".to_string()),
            overlay: Some(stage_overlay(
                agent_core::planner::PlanStageKind::Observe,
                self.id(),
                "applied",
                "👀 采集页面内容",
            )),
        })
    }
}

fn resolve_target_url(
    plan: &AgentPlan,
    context: &crate::agent::stage_context::StageContext,
    prefer_context: bool,
) -> Option<String> {
    if prefer_context {
        if let Some(url) = context.best_known_url() {
            return Some(url);
        }
    }
    let plan_url = plan
        .steps
        .iter()
        .rev()
        .find_map(|step| match &step.tool.kind {
            AgentToolKind::Navigate { url } => Some(url.clone()),
            AgentToolKind::Custom { name, payload }
                if name.eq_ignore_ascii_case("data.extract-site") =>
            {
                payload
                    .as_object()
                    .and_then(|obj| obj.get("url"))
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
            }
            _ => None,
        });
    let candidate = plan_url
        .and_then(|url| Some(adjust_url_for_search(url, context)))
        .or_else(|| context.best_known_url())
        .map(|url| adjust_url_for_search(url, context))
        .or_else(|| Some(context.fallback_search_url()))?;
    Some(normalize_search_results_url(candidate, context))
}

fn adjust_url_for_search(
    url: String,
    context: &crate::agent::stage_context::StageContext,
) -> String {
    if url.contains("?") {
        return url;
    }
    if !context.search_terms.is_empty() {
        return context.fallback_search_url();
    }
    url
}

fn normalize_search_results_url(
    url: String,
    context: &crate::agent::stage_context::StageContext,
) -> String {
    if url.contains("baidu.com/s?") {
        return url;
    }
    if url.contains("baidu.com") && !context.search_terms.is_empty() {
        return context.fallback_search_url();
    }
    url
}
