use agent_core::plan::{AgentPlan, AgentTool, AgentToolKind};
use agent_core::WaitMode;
use serde_json::json;
use std::collections::HashMap;
use url::Url;

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
        let mut steps = Vec::new();
        steps.push(
            StrategyStep::new(
                "打开采集目标",
                AgentTool {
                    kind: AgentToolKind::Navigate { url: url.clone() },
                    wait: WaitMode::DomReady,
                    timeout_ms: Some(20_000),
                },
            )
            .with_detail(format!("跳转至 {url}")),
        );
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
        steps.push(step);
        Some(StrategyApplication {
            steps,
            note: Some("追加 data.extract-site 采集步骤".to_string()),
            overlay: Some(stage_overlay(
                agent_core::planner::PlanStageKind::Observe,
                self.id(),
                "applied",
                "👀 采集页面内容",
            )),
            vendor_context: HashMap::new(),
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
    if host_is_search_engine(&url) {
        if url.contains('?') {
            return url;
        }
        if !context.search_terms.is_empty() {
            return context.fallback_search_url();
        }
    }
    url
}

fn normalize_search_results_url(
    url: String,
    context: &crate::agent::stage_context::StageContext,
) -> String {
    if host_is_search_engine(&url) && !url.contains('?') && !context.search_terms.is_empty() {
        return context.fallback_search_url();
    }
    url
}

fn host_is_search_engine(url: &str) -> bool {
    const SEARCH_SUFFIXES: &[&str] = &[
        "baidu.com",
        "google.com",
        "google.cn",
        "bing.com",
        "yahoo.com",
        "duckduckgo.com",
        "so.com",
        "sogou.com",
    ];

    let host = match Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|h| h.to_ascii_lowercase()))
    {
        Some(host) => host,
        None => return false,
    };
    SEARCH_SUFFIXES
        .iter()
        .any(|suffix| host_matches_suffix(&host, suffix))
}

fn host_matches_suffix(host: &str, suffix: &str) -> bool {
    if host == suffix {
        return true;
    }
    if host.len() <= suffix.len() {
        return false;
    }
    host.ends_with(suffix)
        && host
            .as_bytes()
            .get(host.len() - suffix.len() - 1)
            .copied()
            .map(|ch| ch == b'.')
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::plan::{AgentPlanStep, AgentToolKind};
    use agent_core::{plan::AgentTool, RequestedOutput, WaitMode};
    use soulbrowser_core_types::TaskId;

    fn plan_with_navigation(url: &str) -> AgentPlan {
        let mut plan = AgentPlan::new(TaskId::new(), "test".to_string());
        plan.push_step(AgentPlanStep::new(
            "navigate-1".to_string(),
            "Nav".to_string(),
            AgentTool {
                kind: AgentToolKind::Navigate {
                    url: url.to_string(),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(1_000),
            },
        ));
        plan
    }

    fn context_with_terms(terms: &[&str]) -> crate::agent::stage_context::StageContext {
        crate::agent::stage_context::StageContext {
            current_url: None,
            snapshot_url: None,
            preferred_sites: Vec::new(),
            tenant_default_url: None,
            search_terms: terms.iter().map(|t| t.to_string()).collect(),
            guardrail_keywords: Vec::new(),
            guardrail_keyword_count: 0,
            guardrail_domains: Vec::new(),
            requested_outputs: Vec::<RequestedOutput>::new(),
            browser_context: None,
            search_fallback_url: format!("https://www.baidu.com/s?wd={}", terms[0]),
            force_observe_current: false,
            auto_act_retry: 0,
            auto_act: crate::agent::stage_context::AutoActTuning::default(),
        }
    }

    #[test]
    fn observe_prefers_plan_url_when_not_search_host() {
        let plan = plan_with_navigation("https://quote.eastmoney.com/qh/AG0.html");
        let context = context_with_terms(&["东方财富 白银"]);
        let resolved = resolve_target_url(&plan, &context, false).expect("url");
        assert!(resolved.contains("quote.eastmoney.com"));
    }

    #[test]
    fn observe_uses_search_url_for_search_hosts_without_query() {
        let plan = plan_with_navigation("https://www.google.com");
        let context = context_with_terms(&["白银 走势"]);
        let resolved = resolve_target_url(&plan, &context, false).expect("url");
        assert!(resolved.starts_with("https://www.baidu.com/s?wd="));
    }

    #[test]
    fn search_host_with_query_is_preserved() {
        let plan = plan_with_navigation("https://www.baidu.com/s?wd=白银");
        let context = context_with_terms(&["白银 查询"]);
        let resolved = resolve_target_url(&plan, &context, false).expect("url");
        assert!(resolved.contains("baidu.com/s?wd"));
    }
}
