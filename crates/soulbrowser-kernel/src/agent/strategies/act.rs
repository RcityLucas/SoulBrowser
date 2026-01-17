use agent_core::plan::{
    AgentLocator, AgentPlan, AgentScrollTarget, AgentTool, AgentToolKind, AgentValidation,
    AgentWaitCondition,
};
use agent_core::WaitMode;
use serde_json::{json, Value};
use std::collections::HashMap;
use url::Url;

use super::{stage_overlay, StageStrategy, StrategyApplication, StrategyInput, StrategyStep};
use crate::agent::guardrails::derive_guardrail_domains;
#[cfg(test)]
use crate::agent::stage_context::AutoActTuning;
use crate::agent::stage_context::StageContext;
use crate::agent::SKIP_CLICK_VALIDATION_METADATA_KEY;
use crate::metrics::record_auto_act_search_engine;

#[derive(Debug, Default)]
pub struct AutoActStrategy;

impl AutoActStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for AutoActStrategy {
    fn id(&self) -> &'static str {
        "auto"
    }

    fn stage(&self) -> agent_core::planner::PlanStageKind {
        agent_core::planner::PlanStageKind::Act
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        if let Some(engine) = detect_search_engine(input.plan, input.context) {
            record_auto_act_search_engine(
                input.request.intent.intent_kind.as_str(),
                engine.metric_label(),
            );
            return Some(build_search_application(engine, input));
        }

        Some(build_scroll_application(input))
    }
}

fn build_scroll_application(input: &StrategyInput<'_>) -> StrategyApplication {
    let detail = format!(
        "滚动页面以探索更多关于{}的交互元素",
        input.context.search_seed()
    );
    let tool = AgentTool {
        kind: AgentToolKind::Scroll {
            target: AgentScrollTarget::Pixels(720),
        },
        wait: WaitMode::DomReady,
        timeout_ms: Some(5_000),
    };
    let step = StrategyStep::new("探索可交互区域", tool).with_detail(detail);
    let mut vendor_context = HashMap::new();
    vendor_context.insert(
        "auto_act_engine".to_string(),
        json!({
            "engine": null,
            "label": "scroll_fallback",
            "domain": null,
            "emitted": false,
            "fallback": true,
        }),
    );
    StrategyApplication {
        steps: vec![step],
        note: Some("自动追加滚动动作，确保存在 Act 阶段".to_string()),
        overlay: Some(stage_overlay(
            agent_core::planner::PlanStageKind::Act,
            "auto",
            "applied",
            "🕹️ 自动探索交互区域",
        )),
        vendor_context,
    }
}

fn build_focus_input_step(locator: AgentLocator) -> StrategyStep {
    let mut step = StrategyStep::new(
        "激活搜索框",
        AgentTool {
            kind: AgentToolKind::Click {
                locator: locator.clone(),
            },
            wait: WaitMode::DomReady,
            timeout_ms: Some(10_000),
        },
    )
    .with_detail("点击搜索输入框".to_string());
    step.validations.push(AgentValidation {
        description: "确认停留在搜索页".to_string(),
        condition: AgentWaitCondition::UrlMatches("^https?://".to_string()),
    });
    step.metadata.insert(
        SKIP_CLICK_VALIDATION_METADATA_KEY.to_string(),
        Value::Bool(true),
    );
    step
}

fn build_search_application(
    engine: SearchEngine,
    input: &StrategyInput<'_>,
) -> StrategyApplication {
    let query = input.context.search_seed();
    let mut steps = Vec::new();
    let domain_hints = preferred_result_domains(input);

    if let Some(locator) = engine.focus_locator() {
        steps.push(build_focus_input_step(locator));
    }

    let mut type_step = StrategyStep::new(
        "输入搜索关键词",
        AgentTool {
            kind: AgentToolKind::TypeText {
                locator: engine.input_locator(),
                text: query.clone(),
                submit: engine.submits_with_enter(),
            },
            wait: WaitMode::DomReady,
            timeout_ms: Some(8_000),
        },
    )
    .with_detail(format!("在搜索框输入 {query}"));
    type_step.validations.push(AgentValidation {
        description: "确认停留在搜索页".to_string(),
        condition: AgentWaitCondition::UrlMatches("^https?://".to_string()),
    });
    steps.push(type_step);

    if let Some(submit_step) = engine.submit_step() {
        if let AgentToolKind::Click { .. } = submit_step.tool.kind {
            let mut submit_step = submit_step;
            submit_step.metadata.insert(
                SKIP_CLICK_VALIDATION_METADATA_KEY.to_string(),
                Value::Bool(true),
            );
            steps.push(submit_step);
        } else {
            steps.push(submit_step);
        }
    }

    let result_click = build_search_result_click_step(engine, input, &domain_hints);
    steps.push(result_click);

    let overlay_detail = format!("🕹️ 自动提交{}搜索", engine.label());
    let mut overlay = stage_overlay(
        agent_core::planner::PlanStageKind::Act,
        "auto",
        "applied",
        overlay_detail,
    );
    if let Some(obj) = overlay.as_object_mut() {
        obj.insert(
            "badge".to_string(),
            json!({
                "label": "AutoAct",
                "value": engine.metric_label(),
                "tone": "info",
            }),
        );
        if let Some(domain) = domain_hints.first() {
            obj.insert("target_domain".to_string(), json!(domain));
        }
    }
    let mut vendor_context = HashMap::new();
    vendor_context.insert(
        "auto_act_engine".to_string(),
        json!({
            "engine": engine.metric_label(),
            "label": engine.label(),
            "domains": domain_hints,
            "emitted": false,
        }),
    );
    StrategyApplication {
        steps,
        note: Some(format!(
            "自动填写并提交{}搜索并打开首条结果",
            engine.label()
        )),
        overlay: Some(overlay),
        vendor_context,
    }
}

fn build_search_result_click_step(
    engine: SearchEngine,
    input: &StrategyInput<'_>,
    domain_hints: &[String],
) -> StrategyStep {
    let description = if domain_hints.is_empty() {
        "定位搜索结果".to_string()
    } else {
        format!("打开 {} 的首条搜索结果", domain_hints.join(" / "))
    };
    let selectors = default_result_selectors(&engine);
    let tuning = input.context.auto_act_tuning().clone();
    let max_attempts = tuning.max_attempts.max(1) as u64;
    let wait_per_candidate_ms = tuning.wait_per_candidate_ms.max(1);
    let max_candidates = tuning.max_candidates.max(1);
    let timeout_budget = max_attempts
        .saturating_mul(wait_per_candidate_ms)
        .saturating_add(wait_per_candidate_ms);
    let payload = json!({
        "engine": engine.metric_label(),
        "domains": domain_hints,
        "selectors": selectors,
        "max_attempts": max_attempts,
        "max_candidates": max_candidates,
        "wait_per_candidate_ms": wait_per_candidate_ms,
    });
    let mut step = StrategyStep::new(
        "定位搜索结果",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "browser.search.click-result".to_string(),
                payload,
            },
            wait: WaitMode::DomReady,
            timeout_ms: Some(timeout_budget),
        },
    )
    .with_detail(description);
    if let Some(domain) = domain_hints.first() {
        step.metadata.insert(
            crate::agent::EXPECTED_URL_METADATA_KEY.to_string(),
            serde_json::Value::String(normalize_domain_hint(domain)),
        );
    }
    if tuning.max_refresh_retries > 0 {
        let mut queries = Vec::new();
        for attempt in 0..tuning.max_refresh_retries {
            if let Some(query) = guardrail_refresh_query(input.context, attempt) {
                if queries.last().map_or(true, |existing| existing != &query) {
                    queries.push(query);
                }
            }
        }
        if !queries.is_empty() {
            let max_retries = queries.len().min(tuning.max_refresh_retries as usize) as u32;
            step.metadata.insert(
                "auto_act_refresh".to_string(),
                json!({
                    "engine": engine.metric_label(),
                    "queries": queries,
                    "max_retries": max_retries,
                }),
            );
        }
    }
    step
}
fn preferred_result_domains(input: &StrategyInput<'_>) -> Vec<String> {
    let mut domains = Vec::new();
    for site in input.context.preferred_sites.iter() {
        if let Ok(parsed) = Url::parse(site) {
            if let Some(domain) = parsed.domain() {
                push_unique(&mut domains, domain.to_string());
            }
        }
    }
    for domain in input.context.guardrail_domains.iter() {
        push_unique(&mut domains, domain.to_string());
    }
    if domains.is_empty() {
        for domain in derive_guardrail_domains(input.request) {
            push_unique(&mut domains, domain);
        }
    }
    domains
}

fn guardrail_refresh_query(context: &StageContext, attempt: u32) -> Option<String> {
    let queries = context.guardrail_queries();
    if queries.is_empty() {
        return None;
    }
    if let Some(query) = queries.get(attempt as usize) {
        return Some(query.clone());
    }
    let mut fallback = queries.last()?.clone();
    let seed = context.search_seed();
    if !seed.trim().is_empty() && !fallback.contains(seed.trim()) {
        fallback = format!("{fallback} {seed}");
    }
    if attempt > queries.len() as u32 {
        fallback.push_str(" 最新");
    }
    Some(fallback)
}

fn default_result_selectors(engine: &SearchEngine) -> Vec<String> {
    let selectors: &[&str] = match engine {
        SearchEngine::Baidu => &["div#content_left h3 a", "div#content_left a"],
        SearchEngine::Google => &["div#search h3 a", "div#search a"],
        SearchEngine::Bing => &["ol#b_results li.b_algo h2 a", "ol#b_results li.b_algo a"],
    };
    selectors.iter().map(|s| s.to_string()).collect()
}

fn push_unique(domains: &mut Vec<String>, candidate: String) {
    if !candidate.is_empty()
        && !domains
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&candidate))
    {
        domains.push(candidate);
    }
}

fn detect_search_engine(plan: &AgentPlan, context: &StageContext) -> Option<SearchEngine> {
    browser_search_engine_hint(plan)
        .or_else(|| latest_navigate_url(plan).and_then(|url| SearchEngine::from_url(&url)))
        .or_else(|| {
            let fallback = context.fallback_search_url();
            SearchEngine::from_url(&fallback)
        })
}

fn latest_navigate_url(plan: &AgentPlan) -> Option<String> {
    plan.steps
        .iter()
        .rev()
        .find_map(|step| match &step.tool.kind {
            AgentToolKind::Navigate { url } => Some(url.clone()),
            _ => None,
        })
}

fn browser_search_engine_hint(plan: &AgentPlan) -> Option<SearchEngine> {
    plan.steps
        .iter()
        .rev()
        .find_map(|step| match &step.tool.kind {
            AgentToolKind::Custom { name, payload }
                if name.eq_ignore_ascii_case("browser.search") =>
            {
                if let Some(url) = payload
                    .get("search_url")
                    .and_then(|value| value.as_str())
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                {
                    if let Some(engine) = SearchEngine::from_url(url) {
                        return Some(engine);
                    }
                }
                payload
                    .get("engine")
                    .and_then(|value| value.as_str())
                    .and_then(SearchEngine::from_hint)
            }
            _ => None,
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchEngine {
    Baidu,
    Google,
    Bing,
}

impl SearchEngine {
    fn from_url(url: &str) -> Option<Self> {
        let parsed = Url::parse(url).ok()?;
        let host = parsed.host_str()?.to_ascii_lowercase();
        let path = parsed.path().to_ascii_lowercase();
        if host.contains("baidu.com") && path.starts_with("/s") {
            return Some(Self::Baidu);
        }
        if host.contains("google.") && path.starts_with("/search") {
            return Some(Self::Google);
        }
        if host.contains("bing.com") && path.starts_with("/search") {
            return Some(Self::Bing);
        }
        None
    }

    fn label(&self) -> &'static str {
        match self {
            SearchEngine::Baidu => "百度",
            SearchEngine::Google => "谷歌",
            SearchEngine::Bing => "必应",
        }
    }

    fn metric_label(&self) -> &'static str {
        match self {
            SearchEngine::Baidu => "baidu",
            SearchEngine::Google => "google",
            SearchEngine::Bing => "bing",
        }
    }

    fn from_hint(value: &str) -> Option<Self> {
        let normalized = value.to_ascii_lowercase();
        match normalized.as_str() {
            "baidu" => Some(Self::Baidu),
            "bing" => Some(Self::Bing),
            "google" => Some(Self::Google),
            _ => None,
        }
    }

    fn input_locator(&self) -> AgentLocator {
        match self {
            SearchEngine::Baidu => AgentLocator::Css("input#kw".to_string()),
            SearchEngine::Google => AgentLocator::Css("textarea[name=\"q\"]".to_string()),
            SearchEngine::Bing => AgentLocator::Css("input[name=\"q\"]".to_string()),
        }
    }

    fn submits_with_enter(&self) -> bool {
        !matches!(self, SearchEngine::Baidu)
    }

    fn focus_locator(&self) -> Option<AgentLocator> {
        match self {
            SearchEngine::Baidu => Some(AgentLocator::Css("input#kw".to_string())),
            SearchEngine::Google => Some(AgentLocator::Css("textarea[name=\"q\"]".to_string())),
            SearchEngine::Bing => Some(AgentLocator::Css("input[name=\"q\"]".to_string())),
        }
    }

    fn submit_step(&self) -> Option<StrategyStep> {
        match self {
            SearchEngine::Baidu => Some(build_click_submit_step(
                "提交搜索",
                AgentLocator::Css("input#su".to_string()),
                "点击百度一下提交",
                AgentLocator::Css("div#content_left".to_string()),
            )),
            _ => None,
        }
    }

    #[cfg(test)]
    fn base_result_selector(&self) -> &str {
        match self {
            SearchEngine::Baidu => "div#content_left h3 a",
            SearchEngine::Google => "div#search h3 a",
            SearchEngine::Bing => "ol#b_results li.b_algo h2 a",
        }
    }

    #[cfg(test)]
    fn result_click_locator(&self, domain_hints: &[String]) -> AgentLocator {
        if !domain_hints.is_empty() {
            let mut selectors = Vec::new();
            for hint in domain_hints {
                if let Some(fragment) = domain_fragment(hint) {
                    selectors.push(self.domain_result_selector(&fragment));
                }
            }
            if !selectors.is_empty() {
                let joined = selectors.join(", ");
                return AgentLocator::Css(format!("{}, {}", joined, self.base_result_selector()));
            }
        }
        AgentLocator::Css(self.base_result_selector().to_string())
    }

    #[cfg(test)]
    fn domain_result_selector(&self, fragment: &str) -> String {
        match self {
            SearchEngine::Baidu => css_href_selector("div#content_left h3 a", fragment),
            SearchEngine::Google => css_href_selector("div#search a", fragment),
            SearchEngine::Bing => css_href_selector("ol#b_results li.b_algo a", fragment),
        }
    }
}

fn normalize_domain_hint(domain: &str) -> String {
    let trimmed = domain.trim().trim_start_matches("*").trim_matches('/');
    if trimmed.is_empty() {
        return "https://".to_string();
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if trimmed.starts_with("//") {
        format!("https:{}", trimmed)
    } else {
        format!("https://{}", trimmed)
    }
}

#[cfg(test)]
fn domain_fragment(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(parsed) = Url::parse(trimmed) {
        if let Some(host) = parsed.host_str() {
            return Some(host.trim_start_matches("www.").to_string());
        }
    }
    let without_scheme = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("//");
    let fragment = without_scheme
        .trim_start_matches('*')
        .trim_start_matches('.')
        .trim_matches('/');
    if fragment.is_empty() {
        None
    } else {
        Some(fragment.to_string())
    }
}

#[cfg(test)]
fn css_href_selector(base: &str, fragment: &str) -> String {
    let escaped = fragment.replace('\\', "\\\\").replace('\'', "\\'");
    format!("{}[href*='{}']", base, escaped)
}

fn build_click_submit_step(
    title: &str,
    locator: AgentLocator,
    detail: &str,
    wait_for: AgentLocator,
) -> StrategyStep {
    let mut click_step = StrategyStep::new(
        title,
        AgentTool {
            kind: AgentToolKind::Click { locator },
            wait: WaitMode::Idle,
            timeout_ms: Some(12_000),
        },
    )
    .with_detail(detail.to_string());
    click_step.validations.push(AgentValidation {
        description: "等待结果区域出现".to_string(),
        condition: AgentWaitCondition::ElementVisible(wait_for),
    });
    click_step
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::stage_context::StageContext;
    use agent_core::plan::{AgentPlanStep, AgentTool};
    use agent_core::{AgentIntentKind, AgentRequest, RequestedOutput, WaitMode};
    use serde_json::{json, Value};
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

    fn plan_with_browser_search(engine: Option<&str>, search_url: Option<&str>) -> AgentPlan {
        let mut plan = AgentPlan::new(TaskId::new(), "test-search".to_string());
        let mut payload = json!({ "query": "auto" });
        if let Some(map) = payload.as_object_mut() {
            if let Some(hint) = engine {
                map.insert("engine".to_string(), json!(hint));
            }
            if let Some(url) = search_url {
                map.insert("search_url".to_string(), json!(url));
            }
        }
        plan.push_step(AgentPlanStep::new(
            "search-1".to_string(),
            "Search".to_string(),
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "browser.search".to_string(),
                    payload,
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(1_000),
            },
        ));
        plan
    }

    fn context_with_fallback(url: &str) -> StageContext {
        StageContext {
            current_url: None,
            snapshot_url: None,
            preferred_sites: Vec::new(),
            tenant_default_url: None,
            search_terms: vec!["测试".to_string()],
            guardrail_keywords: Vec::new(),
            guardrail_keyword_count: 0,
            guardrail_domains: Vec::new(),
            requested_outputs: Vec::<RequestedOutput>::new(),
            browser_context: None,
            search_fallback_url: url.to_string(),
            force_observe_current: false,
            auto_act_retry: 0,
            auto_act: AutoActTuning::default(),
        }
    }

    fn guardrail_context() -> StageContext {
        StageContext {
            current_url: None,
            snapshot_url: None,
            preferred_sites: Vec::new(),
            tenant_default_url: None,
            search_terms: vec![
                "通过同花顺帮我查一下今天镍价".to_string(),
                "site:10jqka.com.cn".to_string(),
            ],
            guardrail_keywords: vec!["通过同花顺帮我查一下今天镍价".to_string()],
            guardrail_keyword_count: 1,
            guardrail_domains: vec!["10jqka.com.cn".to_string()],
            requested_outputs: Vec::new(),
            browser_context: None,
            search_fallback_url: "https://www.baidu.com/s?wd=镍价".to_string(),
            force_observe_current: false,
            auto_act_retry: 0,
            auto_act: AutoActTuning::default(),
        }
    }

    #[test]
    fn detects_baidu_search_engine() {
        let plan = plan_with_navigation("https://www.baidu.com/s?wd=测试");
        let context = context_with_fallback("https://www.google.com/search?q=rust");
        assert_eq!(
            detect_search_engine(&plan, &context),
            Some(SearchEngine::Baidu)
        );
    }

    #[test]
    fn detects_google_search_engine() {
        let plan = plan_with_navigation("https://www.google.com/search?q=rust");
        let context = context_with_fallback("https://www.baidu.com/s?wd=应急");
        assert_eq!(
            detect_search_engine(&plan, &context),
            Some(SearchEngine::Google)
        );
    }

    #[test]
    fn detects_bing_search_engine() {
        let plan = plan_with_navigation("https://www.bing.com/search?q=rust");
        let context = context_with_fallback("https://www.baidu.com/s?wd=应急");
        assert_eq!(
            detect_search_engine(&plan, &context),
            Some(SearchEngine::Bing)
        );
    }

    #[test]
    fn detects_engine_from_browser_search_payload() {
        let plan = plan_with_browser_search(Some("bing"), None);
        let context = context_with_fallback("https://www.baidu.com/s?wd=兜底");
        assert_eq!(
            detect_search_engine(&plan, &context),
            Some(SearchEngine::Bing)
        );
    }

    #[test]
    fn detects_engine_from_browser_search_url_override() {
        let plan = plan_with_browser_search(None, Some("https://www.google.com/search?q=auto"));
        let context = context_with_fallback("https://www.baidu.com/s?wd=兜底");
        assert_eq!(
            detect_search_engine(&plan, &context),
            Some(SearchEngine::Google)
        );
    }

    #[test]
    fn detects_engine_from_context_fallback() {
        let plan = AgentPlan::new(TaskId::new(), "empty".to_string());
        let context = context_with_fallback("https://www.google.com/search?q=从兜底检测");
        assert_eq!(
            detect_search_engine(&plan, &context),
            Some(SearchEngine::Google)
        );
    }

    #[test]
    fn guardrail_refresh_queries_cycle_through_variants() {
        let context = guardrail_context();
        let first = guardrail_refresh_query(&context, 0).expect("first query");
        assert!(first.contains("site:10jqka.com.cn"));
        let second = guardrail_refresh_query(&context, 1).expect("second query");
        assert!(second.contains("10jqka.com.cn"));
        assert_ne!(first, second);
        let third = guardrail_refresh_query(&context, 2).expect("third query");
        assert!(third.contains("行情") || third.contains("报价"));
        let fourth = guardrail_refresh_query(&context, 3).expect("fallback query");
        assert!(fourth.contains("最新") || fourth.contains("镍价"));
    }

    #[test]
    fn click_step_sets_expected_url_from_guardrail() {
        let mut plan = plan_with_navigation("https://www.baidu.com/s?wd=测试");
        let mut request = AgentRequest::new(TaskId::new(), "guardrail test");
        request.intent.intent_kind = AgentIntentKind::Informational;
        request.intent.target_sites = vec!["https://stats.gov.cn".to_string()];
        let context = StageContext {
            current_url: None,
            snapshot_url: None,
            preferred_sites: request.intent.target_sites.clone(),
            tenant_default_url: None,
            search_terms: vec!["测试".to_string()],
            guardrail_keywords: request.intent.target_sites.clone(),
            guardrail_keyword_count: request.intent.target_sites.len(),
            guardrail_domains: vec!["stats.gov.cn".to_string()],
            requested_outputs: Vec::<RequestedOutput>::new(),
            browser_context: None,
            search_fallback_url: "https://www.baidu.com/s?wd=测试".to_string(),
            force_observe_current: false,
            auto_act_retry: 0,
            auto_act: AutoActTuning::default(),
        };
        let strategy = AutoActStrategy::new();
        let application = strategy
            .apply(&StrategyInput {
                plan: &mut plan,
                request: &request,
                context: &context,
            })
            .expect("auto act");
        let click_step = application
            .steps
            .iter()
            .find(|step| match &step.tool.kind {
                AgentToolKind::Custom { name, .. } => name == "browser.search.click-result",
                _ => false,
            })
            .expect("result click step");
        let expected = click_step
            .metadata
            .get(crate::agent::EXPECTED_URL_METADATA_KEY)
            .and_then(Value::as_str)
            .expect("expected url");
        assert!(expected.contains("stats.gov.cn"));
    }

    #[test]
    fn preferred_domains_include_preferred_sites_and_guardrails() {
        let mut plan = plan_with_navigation("https://www.baidu.com");
        let mut request = AgentRequest::new(TaskId::new(), "guardrail domain test");
        request.intent.target_sites = vec!["https://quote.eastmoney.com".to_string()];
        request.intent.validation_keywords = vec!["白银 新浪".to_string()];
        let context = StageContext {
            current_url: None,
            snapshot_url: None,
            preferred_sites: request.intent.target_sites.clone(),
            tenant_default_url: None,
            search_terms: vec!["白银".to_string()],
            guardrail_keywords: vec!["白银 新浪".to_string()],
            guardrail_keyword_count: 1,
            guardrail_domains: vec!["quote.eastmoney.com".to_string()],
            requested_outputs: Vec::new(),
            browser_context: None,
            search_fallback_url: "https://www.baidu.com".to_string(),
            force_observe_current: false,
            auto_act_retry: 0,
            auto_act: AutoActTuning::default(),
        };
        let input = StrategyInput {
            plan: &mut plan,
            request: &request,
            context: &context,
        };
        let domains = preferred_result_domains(&input);
        assert!(domains.iter().any(|d| d.contains("quote.eastmoney.com")));
        assert!(domains.len() >= 1);
    }

    #[test]
    fn result_locator_combines_multiple_domains() {
        let domains = vec![
            "https://quote.eastmoney.com".to_string(),
            "finance.sina.com.cn".to_string(),
        ];
        let locator = SearchEngine::Baidu.result_click_locator(&domains);
        match locator {
            AgentLocator::Css(selector) => {
                assert!(selector.contains("href*='quote.eastmoney.com'"));
                assert!(selector.contains("href*='finance.sina.com.cn'"));
                assert!(selector.contains("div#content_left h3 a"));
            }
            _ => panic!("expected CSS locator"),
        }
    }

    #[test]
    fn domain_fragment_parses_urls_and_wildcards() {
        assert_eq!(
            domain_fragment("https://quote.eastmoney.com"),
            Some("quote.eastmoney.com".to_string())
        );
        assert_eq!(
            domain_fragment("*.chinacourt.gov.cn"),
            Some("chinacourt.gov.cn".to_string())
        );
        assert_eq!(domain_fragment(""), None);
    }

    #[test]
    fn baidu_locator_prefers_guardrail_domain() {
        let domains = vec!["https://quote.eastmoney.com".to_string()];
        match SearchEngine::Baidu.result_click_locator(&domains) {
            AgentLocator::Css(selector) => {
                assert!(selector.contains("href*='quote.eastmoney.com'"));
            }
            _ => panic!("expected css locator"),
        }
    }
}
