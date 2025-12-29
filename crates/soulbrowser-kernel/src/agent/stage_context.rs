use agent_core::{
    requires_weather_pipeline, weather_query_text, weather_search_url, AgentIntentKind,
    AgentRequest, RequestedOutput,
};
use serde_json::Value;
use std::env;
use url::form_urlencoded;

#[derive(Debug, Clone)]
pub struct StageContext {
    pub current_url: Option<String>,
    pub snapshot_url: Option<String>,
    pub preferred_sites: Vec<String>,
    pub tenant_default_url: Option<String>,
    pub search_terms: Vec<String>,
    pub requested_outputs: Vec<RequestedOutput>,
    pub browser_context: Option<Value>,
    pub search_fallback_url: String,
    pub force_observe_current: bool,
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
        let terms = self.search_terms(weather_required);
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
            search_terms: terms.clone(),
            requested_outputs: self.request.intent.required_outputs.clone(),
            browser_context,
            search_fallback_url: if weather_required {
                weather_search_url(self.request)
            } else {
                make_search_url(
                    terms
                        .first()
                        .cloned()
                        .unwrap_or_else(|| self.request.goal.clone()),
                    self.request,
                )
            },
            force_observe_current: self.force_observe_current(),
        }
    }

    fn current_url(&self) -> Option<String> {
        self.request
            .context
            .as_ref()
            .and_then(|ctx| ctx.current_url.as_deref())
            .map(|value| value.to_string())
    }

    fn search_terms(&self, weather_required: bool) -> Vec<String> {
        let mut terms = Vec::new();
        if weather_required {
            push_term(&mut terms, &weather_query_text(self.request));
        }
        if let Some(goal) = self.request.intent.primary_goal.as_deref() {
            push_term(&mut terms, goal);
        }
        push_term(&mut terms, self.request.goal.as_str());
        if let Some(Value::Array(extra)) = self.request.metadata.get("search_terms") {
            for item in extra {
                if let Some(s) = item.as_str() {
                    push_term(&mut terms, s);
                }
            }
        }
        if terms.is_empty() {
            terms.push("网页采集".to_string());
        }
        terms
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
