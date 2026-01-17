use crate::agent::guardrails::{derive_guardrail_domains, derive_guardrail_keywords};
use crate::metrics::record_guardrail_keyword_usage;
use agent_core::{
    requires_weather_pipeline, weather_query_text, weather_search_url, AgentIntentKind,
    AgentRequest, RequestedOutput,
};
use serde_json::Value;
#[cfg(test)]
use soulbrowser_core_types::TaskId;
use std::collections::HashSet;
use std::env;
use url::form_urlencoded;

#[derive(Debug, Clone)]
pub struct StageContext {
    pub current_url: Option<String>,
    pub snapshot_url: Option<String>,
    pub preferred_sites: Vec<String>,
    pub tenant_default_url: Option<String>,
    pub search_terms: Vec<String>,
    pub guardrail_keywords: Vec<String>,
    pub guardrail_keyword_count: usize,
    pub guardrail_domains: Vec<String>,
    pub requested_outputs: Vec<RequestedOutput>,
    pub browser_context: Option<Value>,
    pub search_fallback_url: String,
    pub force_observe_current: bool,
    pub auto_act_retry: u32,
    pub auto_act: AutoActTuning,
}

#[derive(Debug, Clone)]
pub struct AutoActTuning {
    pub max_attempts: u32,
    pub max_candidates: u32,
    pub wait_per_candidate_ms: u64,
    pub max_refresh_retries: u32,
}

impl Default for AutoActTuning {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            max_candidates: 40,
            wait_per_candidate_ms: 15_000,
            max_refresh_retries: 3,
        }
    }
}

impl StageContext {
    pub fn best_known_url(&self) -> Option<String> {
        self.current_url
            .clone()
            .or_else(|| self.snapshot_url.clone())
            .or_else(|| self.tenant_default_url.clone())
    }

    pub fn search_seed(&self) -> String {
        self.search_terms
            .first()
            .cloned()
            .unwrap_or_else(|| "agent task".to_string())
    }

    pub fn fallback_search_url(&self) -> String {
        self.search_fallback_url.clone()
    }

    pub fn should_force_current_observation(&self) -> bool {
        self.force_observe_current
    }

    pub fn auto_act_tuning(&self) -> &AutoActTuning {
        &self.auto_act
    }

    pub fn guardrail_queries(&self) -> Vec<String> {
        let (keyword_terms, site_terms) = split_search_terms(&self.search_terms);
        let mut base_keywords = if keyword_terms.is_empty() {
            vec![self.search_seed()]
        } else {
            keyword_terms.clone()
        };
        if base_keywords.is_empty() {
            base_keywords.push(self.search_seed());
        }
        let keyword_slice: Vec<String> = base_keywords.iter().take(2).cloned().collect::<Vec<_>>();
        let mut queries = Vec::new();
        let mut seen = HashSet::new();

        for site in site_terms.iter().take(2) {
            if let Some(query) = compose_query(keyword_slice.clone(), vec![site.clone()]) {
                if seen.insert(query.to_ascii_lowercase()) {
                    queries.push(query);
                }
            }
        }

        let domain_aliases = guardrail_domain_aliases(&self.guardrail_domains);
        if !domain_aliases.is_empty() {
            let extras: Vec<String> = domain_aliases.into_iter().take(2).collect();
            if let Some(query) = compose_query(keyword_slice.clone(), extras) {
                if seen.insert(query.to_ascii_lowercase()) {
                    queries.push(query);
                }
            }
        }

        let fallback_tokens = fallback_keyword_hints(&base_keywords);
        if !fallback_tokens.is_empty() {
            let mut fallback_base = vec![self.search_seed()];
            fallback_base.extend(fallback_tokens.into_iter().take(2));
            if let Some(query) = compose_query(Vec::new(), fallback_base) {
                if seen.insert(query.to_ascii_lowercase()) {
                    queries.push(query);
                }
            }
        }

        if let Some(query) = compose_query(keyword_slice.clone(), Vec::new()) {
            if seen.insert(query.to_ascii_lowercase()) {
                queries.push(query);
            }
        }

        if queries.is_empty() {
            if let Some(query) = compose_query(vec![self.search_seed()], Vec::new()) {
                if seen.insert(query.to_ascii_lowercase()) {
                    queries.push(query);
                }
            }
        }

        queries
    }
}

pub struct ContextResolver<'a> {
    request: &'a AgentRequest,
}

impl<'a> ContextResolver<'a> {
    pub fn new(request: &'a AgentRequest) -> Self {
        Self { request }
    }

    pub fn build(&self) -> StageContext {
        let browser_context = self.request.metadata.get("browser_context").cloned();
        let weather_required = requires_weather_pipeline(self.request);
        let search_terms = self.search_terms(weather_required);
        if !search_terms.guardrail_keywords.is_empty() {
            record_guardrail_keyword_usage(
                self.request.intent.intent_kind.as_str(),
                search_terms.guardrail_keywords.len(),
            );
        }
        StageContext {
            current_url: self.current_url(),
            snapshot_url: browser_context
                .as_ref()
                .and_then(|value| extract_nested_url(value)),
            preferred_sites: self.request.intent.target_sites.clone(),
            tenant_default_url: self
                .request
                .metadata
                .get("tenant_default_url")
                .and_then(Value::as_str)
                .map(|value| value.to_string()),
            search_terms: search_terms.terms.clone(),
            guardrail_keywords: search_terms.guardrail_keywords.clone(),
            guardrail_keyword_count: search_terms.guardrail_keywords.len(),
            guardrail_domains: derive_guardrail_domains(self.request),
            requested_outputs: self.request.intent.required_outputs.clone(),
            browser_context,
            search_fallback_url: if weather_required {
                weather_search_url(self.request)
            } else {
                make_search_url(
                    search_terms
                        .terms
                        .first()
                        .cloned()
                        .unwrap_or_else(|| self.request.goal.clone()),
                    self.request,
                )
            },
            force_observe_current: self.force_observe_current(),
            auto_act_retry: self
                .request
                .metadata
                .get("auto_act_retry")
                .and_then(Value::as_u64)
                .map(|value| value as u32)
                .unwrap_or(0),
            auto_act: self.auto_act_tuning(),
        }
    }

    fn auto_act_tuning(&self) -> AutoActTuning {
        let mut tuning = AutoActTuning::default();
        if let Some(Value::Object(config)) = self.request.metadata.get("auto_act") {
            if let Some(value) = config.get("max_attempts").and_then(Value::as_u64) {
                tuning.max_attempts = value.max(1) as u32;
            }
            if let Some(value) = config.get("max_candidates").and_then(Value::as_u64) {
                tuning.max_candidates = value.max(1) as u32;
            }
            if let Some(value) = config.get("wait_per_candidate_ms").and_then(Value::as_u64) {
                tuning.wait_per_candidate_ms = value.max(1);
            }
            if let Some(value) = config.get("max_refresh_retries").and_then(Value::as_u64) {
                tuning.max_refresh_retries = value as u32;
            }
        }
        tuning
    }

    fn current_url(&self) -> Option<String> {
        self.request
            .context
            .as_ref()
            .and_then(|ctx| ctx.current_url.as_deref())
            .map(|value| value.to_string())
    }

    fn search_terms(&self, weather_required: bool) -> SearchTermsResult {
        let mut terms = Vec::new();
        let guardrail_keywords = derive_guardrail_keywords(self.request);
        let guardrail_domains = derive_guardrail_domains(self.request);
        if let Some(Value::Array(extra)) = self.request.metadata.get("search_terms") {
            for item in extra {
                if let Some(s) = item.as_str() {
                    push_term(&mut terms, s);
                }
            }
        }
        if weather_required {
            push_term(&mut terms, &weather_query_text(self.request));
        }
        if let Some(goal) = self.request.intent.primary_goal.as_deref() {
            push_term(&mut terms, goal);
        }
        push_term(&mut terms, self.request.goal.as_str());
        for keyword in &guardrail_keywords {
            push_term(&mut terms, &keyword);
        }
        for domain in &guardrail_domains {
            push_term(&mut terms, &format!("site:{domain}"));
        }
        if terms.is_empty() {
            terms.push("网页采集".to_string());
        }
        SearchTermsResult {
            terms,
            guardrail_keywords,
        }
    }

    fn force_observe_current(&self) -> bool {
        let execute_requested = self
            .request
            .metadata
            .get("execute_requested")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        execute_requested
            && matches!(
                self.request.intent.intent_kind,
                AgentIntentKind::Informational
            )
    }
}

fn make_search_url(seed: String, request: &AgentRequest) -> String {
    let encoded: String = form_urlencoded::byte_serialize(seed.as_bytes()).collect();
    let base = resolve_search_base(request);
    if base.contains("{query}") {
        return base.replace("{query}", &encoded);
    }
    if base.contains("%s") {
        return base.replace("%s", &encoded);
    }
    if base.ends_with('=') || base.ends_with('?') || base.ends_with('&') {
        return format!("{}{}", base, encoded);
    }
    if base.contains('?') {
        return format!("{}&q={}", base, encoded);
    }
    if base.ends_with('/') {
        return format!("{}{}", base, encoded);
    }
    format!("{}/{}", base.trim_end_matches('/'), encoded)
}

fn resolve_search_base(request: &AgentRequest) -> String {
    if let Some(base) = request
        .metadata
        .get("search_base_url")
        .and_then(Value::as_str)
        .map(|value| value.to_string())
    {
        return base;
    }
    if let Ok(base) = env::var("SOUL_SEARCH_BASE_URL") {
        if !base.trim().is_empty() {
            return base;
        }
    }
    "https://www.baidu.com/s?wd=".to_string()
}

fn extract_nested_url(value: &Value) -> Option<String> {
    if let Some(url) = value.get("page").and_then(|page| page.get("url")) {
        if let Some(url_str) = url.as_str() {
            return Some(url_str.to_string());
        }
    }
    if let Some(url) = value.get("current_url").and_then(Value::as_str) {
        return Some(url.to_string());
    }
    value
        .pointer("/perception/page/url")
        .and_then(Value::as_str)
        .map(|value| value.to_string())
}

fn push_term(terms: &mut Vec<String>, candidate: &str) {
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return;
    }
    if terms.iter().any(|existing| existing == trimmed) {
        return;
    }
    terms.push(trimmed.to_string());
}

const FALLBACK_KEYWORD_HINTS: &[&str] = &["行情", "报价", "走势", "价格", "最新"];

fn split_search_terms(terms: &[String]) -> (Vec<String>, Vec<String>) {
    let mut keyword_terms = Vec::new();
    let mut site_terms = Vec::new();
    let mut keyword_seen = HashSet::new();
    let mut site_seen = HashSet::new();
    for term in terms {
        let trimmed = term.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("site:") {
            let normalized = trimmed.to_ascii_lowercase();
            if site_seen.insert(normalized) {
                site_terms.push(trimmed.to_string());
            }
            continue;
        }
        let normalized = trimmed.to_ascii_lowercase();
        if keyword_seen.insert(normalized) {
            keyword_terms.push(trimmed.to_string());
        }
    }
    (keyword_terms, site_terms)
}

fn guardrail_domain_aliases(domains: &[String]) -> Vec<String> {
    let mut aliases = Vec::new();
    let mut seen = HashSet::new();
    for domain in domains {
        let normalized = domain
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("//")
            .trim_matches('/');
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.to_ascii_lowercase()) {
            aliases.push(normalized.to_string());
        }
        if let Some(first) = normalized.split('.').next() {
            if !first.is_empty() {
                let lowered = first.to_ascii_lowercase();
                if seen.insert(lowered) {
                    aliases.push(first.to_string());
                }
            }
        }
    }
    aliases
}

fn fallback_keyword_hints(existing: &[String]) -> Vec<String> {
    let mut hints = Vec::new();
    for hint in FALLBACK_KEYWORD_HINTS {
        if existing.iter().any(|kw| kw.contains(hint)) {
            continue;
        }
        hints.push(hint.to_string());
        if hints.len() >= 2 {
            break;
        }
    }
    hints
}

fn compose_query(mut base: Vec<String>, extras: Vec<String>) -> Option<String> {
    base.extend(extras);
    let tokens: Vec<String> = base
        .into_iter()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn metadata_search_terms_take_precedence() {
        let mut request = AgentRequest::new(TaskId::new(), "查询白银走势");
        request.metadata.insert(
            "search_terms".to_string(),
            json!(["东方财富 白银", "新浪 白银"]),
        );
        request.intent.primary_goal = Some("查银价".to_string());

        let context = ContextResolver::new(&request).build();
        assert_eq!(context.search_terms[0], "东方财富 白银");
        assert!(context
            .search_terms
            .iter()
            .any(|term| term.contains("查银价")));
    }

    #[test]
    fn guardrail_keywords_extend_search_terms() {
        let mut request = AgentRequest::new(TaskId::new(), "我想看最高人民检察院发布的权威数据");
        request.intent.validation_keywords = vec!["最高人民检察院 数据".to_string()];
        request.intent.allowed_domains = vec!["https://10jqka.com.cn".to_string()];
        let context = ContextResolver::new(&request).build();
        assert!(context
            .search_terms
            .iter()
            .any(|term| term.contains("最高人民检察院 数据")));
        assert!(context.guardrail_keyword_count >= 1);
        assert!(context
            .guardrail_keywords
            .iter()
            .any(|kw| kw.contains("最高人民检察院")));
        assert!(context
            .search_terms
            .iter()
            .any(|term| term.contains("site:10jqka.com.cn")));
    }

    #[test]
    fn guardrail_queries_provide_multi_phase_keywords() {
        let context = StageContext {
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
        };
        let queries = context.guardrail_queries();
        assert!(!queries.is_empty());
        assert!(queries
            .iter()
            .any(|query| query.contains("site:10jqka.com.cn")));
        assert!(queries
            .iter()
            .any(|query| query.contains("10jqka.com.cn") && !query.contains("site:")));
        assert!(queries
            .iter()
            .any(|query| query.contains("行情") || query.contains("报价")));
    }
}

#[derive(Debug, Clone)]
struct SearchTermsResult {
    terms: Vec<String>,
    guardrail_keywords: Vec<String>,
}
