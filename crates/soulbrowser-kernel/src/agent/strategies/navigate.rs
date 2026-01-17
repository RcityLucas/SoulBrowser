use agent_core::{
    plan::{AgentTool, AgentToolKind},
    requires_weather_pipeline, WaitMode,
};
use serde_json::{json, Value};
use std::collections::HashMap;

use super::{stage_overlay, StageStrategy, StrategyApplication, StrategyInput, StrategyStep};

fn navigate_step(title: &str, url: &str) -> StrategyStep {
    let tool = AgentTool {
        kind: AgentToolKind::Navigate {
            url: url.to_string(),
        },
        wait: WaitMode::DomReady,
        timeout_ms: Some(30_000),
    };
    StrategyStep::new(title, tool).with_detail(format!("前往 {url}"))
}

#[derive(Debug, Default)]
pub struct ContextUrlNavigateStrategy;

impl ContextUrlNavigateStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for ContextUrlNavigateStrategy {
    fn id(&self) -> &'static str {
        "context_url"
    }

    fn stage(&self) -> agent_core::planner::PlanStageKind {
        agent_core::planner::PlanStageKind::Navigate
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        let url = input
            .context
            .best_known_url()
            .unwrap_or_else(|| input.context.fallback_search_url());
        Some(StrategyApplication {
            steps: vec![navigate_step("导航至当前页面", &url)],
            note: Some(format!("复用上下文 URL {url}")),
            overlay: {
                let mut overlay = stage_overlay(
                    agent_core::planner::PlanStageKind::Navigate,
                    self.id(),
                    "applied",
                    "🏗️ 复用当前页面",
                );
                if let Some(obj) = overlay.as_object_mut() {
                    obj.insert("url".to_string(), Value::String(url));
                }
                Some(overlay)
            },
            vendor_context: HashMap::new(),
        })
    }
}

#[derive(Debug, Default)]
pub struct PreferredSiteNavigateStrategy;

impl PreferredSiteNavigateStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for PreferredSiteNavigateStrategy {
    fn id(&self) -> &'static str {
        "preferred_site"
    }

    fn stage(&self) -> agent_core::planner::PlanStageKind {
        agent_core::planner::PlanStageKind::Navigate
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        let preferred = if requires_weather_pipeline(input.request) {
            preferred_weather_site(&input.context.preferred_sites)
        } else {
            None
        };
        let first = preferred.or_else(|| {
            input
                .context
                .preferred_sites
                .iter()
                .find_map(|site| normalize_url(site))
        });
        let Some(url) = first else {
            return None;
        };
        Some(StrategyApplication {
            steps: vec![navigate_step("打开首选站点", &url)],
            note: Some(format!("优先访问 {url}")),
            overlay: {
                let mut overlay = stage_overlay(
                    agent_core::planner::PlanStageKind::Navigate,
                    self.id(),
                    "applied",
                    "🌐 跳转到首选站点",
                );
                if let Some(obj) = overlay.as_object_mut() {
                    obj.insert("url".to_string(), Value::String(url));
                }
                Some(overlay)
            },
            vendor_context: HashMap::new(),
        })
    }
}

#[derive(Debug, Default)]
pub struct SearchNavigateStrategy;

impl SearchNavigateStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for SearchNavigateStrategy {
    fn id(&self) -> &'static str {
        "search"
    }

    fn stage(&self) -> agent_core::planner::PlanStageKind {
        agent_core::planner::PlanStageKind::Navigate
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        let query = input
            .context
            .guardrail_queries()
            .into_iter()
            .next()
            .unwrap_or_else(|| input.context.search_seed());
        let payload = json!({ "query": query });
        let tool = AgentTool {
            kind: AgentToolKind::Custom {
                name: "browser.search".to_string(),
                payload,
            },
            wait: WaitMode::DomReady,
            timeout_ms: Some(30_000),
        };
        let step = StrategyStep::new("搜索相关网页", tool).with_detail("自动触发浏览器搜索");
        Some(StrategyApplication {
            steps: vec![step],
            note: Some("插入 browser.search 搜索入口".to_string()),
            overlay: {
                let mut overlay = stage_overlay(
                    agent_core::planner::PlanStageKind::Navigate,
                    self.id(),
                    "applied",
                    "🔍 使用搜索策略",
                );
                if let Some(obj) = overlay.as_object_mut() {
                    obj.insert("query".to_string(), Value::String(query));
                }
                Some(overlay)
            },
            vendor_context: HashMap::new(),
        })
    }
}

#[derive(Debug, Default)]
pub struct WeatherSearchStrategy;

impl WeatherSearchStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for WeatherSearchStrategy {
    fn id(&self) -> &'static str {
        "weather_search"
    }

    fn stage(&self) -> agent_core::planner::PlanStageKind {
        agent_core::planner::PlanStageKind::Navigate
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        if !requires_weather_pipeline(input.request) {
            return None;
        }
        let query = input
            .context
            .search_terms
            .first()
            .cloned()
            .unwrap_or_else(|| input.request.goal.clone());
        let tool = AgentTool {
            kind: AgentToolKind::Custom {
                name: "weather.search".to_string(),
                payload: json!({
                    "query": query,
                    "result_selector": "div#content_left"
                }),
            },
            wait: WaitMode::None,
            timeout_ms: Some(30_000),
        };
        let step =
            StrategyStep::new("自动天气搜索", tool).with_detail("封装天气搜索流程，等待结果就绪");
        Some(StrategyApplication {
            steps: vec![step],
            note: Some("插入 weather.search 宏工具".to_string()),
            overlay: {
                let mut overlay = stage_overlay(
                    agent_core::planner::PlanStageKind::Navigate,
                    self.id(),
                    "applied",
                    "🌦️ 使用天气搜索宏工具",
                );
                if let Some(obj) = overlay.as_object_mut() {
                    obj.insert("query".to_string(), Value::String(query));
                }
                Some(overlay)
            },
            vendor_context: HashMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::stage_context::ContextResolver;
    use agent_core::{AgentPlan, AgentRequest};
    use soulbrowser_core_types::TaskId;

    fn weather_request() -> AgentRequest {
        AgentRequest::new(TaskId::new(), "查询济南天气")
    }

    #[test]
    fn search_strategy_uses_guardrail_domains() {
        let mut request = AgentRequest::new(TaskId::new(), "通过同花顺帮我查镍价");
        request.intent.allowed_domains = vec!["https://www.10jqka.com.cn".to_string()];
        request.intent.validation_keywords = vec!["同花顺 镍价".to_string()];
        let context = ContextResolver::new(&request).build();
        let plan = AgentPlan::new(TaskId::new(), "demo");
        let input = StrategyInput {
            plan: &plan,
            request: &request,
            context: &context,
        };
        let strategy = SearchNavigateStrategy::new();
        let application = strategy.apply(&input).expect("applied");
        let payload = match &application.steps[0].tool.kind {
            AgentToolKind::Custom { payload, .. } => payload.clone(),
            other => panic!("unexpected tool: {other:?}"),
        };
        let query = payload
            .get("query")
            .and_then(Value::as_str)
            .expect("query present");
        assert!(query.contains("site:10jqka.com.cn"));
        assert!(query.contains("同花顺"));
    }

    #[test]
    fn weather_strategy_emits_macro_step() {
        let request = weather_request();
        let context = ContextResolver::new(&request).build();
        let plan = AgentPlan::new(TaskId::new(), "demo");
        let input = StrategyInput {
            plan: &plan,
            request: &request,
            context: &context,
        };
        let strategy = WeatherSearchStrategy::new();
        let application = strategy.apply(&input).expect("applied");
        assert_eq!(application.steps.len(), 1);
        match &application.steps[0].tool.kind {
            AgentToolKind::Custom { name, payload } => {
                assert_eq!(name, "weather.search");
                assert!(payload.get("query").is_some());
            }
            other => panic!("unexpected tool: {other:?}"),
        }
    }
}

fn normalize_url(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Some(trimmed.to_string())
    } else {
        Some(format!("https://{}", trimmed.trim_matches('/')))
    }
}

fn preferred_weather_site(sites: &[String]) -> Option<String> {
    sites.iter().find_map(|site| {
        let lowered = site.to_ascii_lowercase();
        if lowered.contains("weather") {
            normalize_url(site)
        } else {
            None
        }
    })
}
