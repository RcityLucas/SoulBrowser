//! Browser automation tools module
#![allow(dead_code)]
//!
//! Provides browser automation tools using soulbase-tools

use crate::block_detect::detect_block_reason;
use crate::errors::SoulBrowserError;
use crate::metrics::{record_market_quote_fallback, record_market_quote_fetch};
use crate::parsers::{
    parse_facebook_feed, parse_github_repos, parse_hackernews_feed, parse_linkedin_profile,
    parse_market_info, parse_metal_price, parse_news_brief, parse_twitter_feed, parse_weather,
};
use crate::self_heal::{record_event as record_self_heal_event, SelfHealEvent};
use crate::structured_output::{
    summarize_structured_output, validate_metal_price_with_context, validate_structured_output,
    MetalPriceValidationContext, MetalPriceValidationFailure,
};
use action_primitives::{
    ActionError, ActionPrimitives, AnchorDescriptor, DefaultActionPrimitives, DefaultWaitStrategy,
    ExecCtx, ScrollBehavior, ScrollTarget, SelectMethod, WaitCondition, WaitTier,
};
use agent_core::planner::mark_source_unhealthy;
use cdp_adapter::{event_bus, AdapterError, AdapterMode, Cdp, CdpAdapter, CdpConfig};
use chrono::Utc;
use dashmap::DashMap;
use l6_observe::{guard::LabelMap as ObsLabelMap, metrics as obs_metrics, tracing as obs_tracing};
use once_cell::sync::Lazy;
use regex::escape;
use reqwest::Client;
use schemars::schema::{RootSchema, SchemaObject};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use soulbase_tools::{
    manifest::{
        CapabilityDecl, ConcurrencyKind, ConsentPolicy, IdempoKind, Limits, SafetyClass,
        SideEffect, ToolId, ToolManifest,
    },
    registry::{AvailableSpec, ListFilter},
    InMemoryRegistry, ToolRegistry,
};
use soulbase_types::tenant::TenantId;
use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId};
use soulbrowser_policy_center::{default_snapshot, PolicyView};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use url::{form_urlencoded, Url};
use uuid::Uuid;

const METRIC_TOOL_INVOCATIONS: &str = "soul.l5.tool.invocations";
const METRIC_TOOL_LATENCY: &str = "soul.l5.tool.latency_ms";
const DEFAULT_TOOL_TIMEOUT_MS: u64 = 30_000;
const PAGE_OBSERVE_SCRIPT: &str = include_str!("scripts/page_observe.js");
const QUOTE_FETCH_SCRIPT: &str = include_str!("scripts/quote_fetch.js");
const WEATHER_CANDIDATE_LIMIT: usize = 5;
const WEATHER_BLOCKED_CAUSE: &str = "WeatherLinkBlocked";
const SEARCH_RESULT_ATTR: &str = "data-soulbrowser-search-hit";
const BAIDU_RESULT_REDIRECT_PATTERN: &str = r"^https?://www\.baidu\.com/link.*";
const AUTO_ACT_RESULT_PICKER_SCRIPT: &str = include_str!("scripts/auto_act_result_picker.js");
const BAIDU_SEARCH_SELECTORS: &[&str] = &[
    "div#content_left .c-container", // Modern Baidu result container (most reliable)
    "div#content_left h3",           // Classic result title
    "div#content_left .result",      // Legacy result class
    "#content_left",                 // Content container itself
    "div#wrapper",                   // Page wrapper fallback
    "div.result",                    // Generic result
    "#page",                         // Pagination (indicates page loaded)
    ".nors",                         // No results indicator
];
const BING_SEARCH_SELECTORS: &[&str] = &["main#b_content", "ol#b_results", "div#b_content"];
const GOOGLE_SEARCH_SELECTORS: &[&str] = &["div#search", "div#center_col", "div.g"];
// DuckDuckGo has fewer captchas than other search engines
const DUCKDUCKGO_SEARCH_SELECTORS: &[&str] = &[
    "div.results",
    "article[data-testid='result']",
    "div#links",
    "div.result",
];
const DEFAULT_MODAL_CLOSE_SELECTORS: &[&str] = &[
    "button[aria-label='Close']",
    "button[aria-label='关闭']",
    ".modal-close",
    ".modal__close",
    ".close-button",
    ".close-btn",
    ".ant-modal-close",
    ".el-dialog__headerbtn",
    ".tj_close",
];
const TRUSTED_WEATHER_DOMAINS: &[&str] = &[
    "moji.com",
    "tianqi.com",
    "weather.com.cn",
    "weather.com",
    "weathercn.com",
    "tianqi365.com",
    "tianqi114.com",
];
const DATA_PARSE_TOOLS: &[(&str, &str)] = &[
    ("data.parse.generic", "Parse generic observation snapshot"),
    ("data.parse.market_info", "Parse market index snapshot"),
    (
        "data.parse.metal_price",
        "Parse metal price quotes into structured output",
    ),
    (
        "data.validate.metal_price",
        "Validate metal price structured data freshness",
    ),
    (
        "data.validate-target",
        "Validate observed page against target keywords and domains",
    ),
    ("data.parse.news_brief", "Parse news brief"),
    (
        "data.parse.weather",
        "Parse weather widget into structured report",
    ),
    ("data.parse.twitter-feed", "Parse Twitter/X feed"),
    ("data.parse.facebook-feed", "Parse Facebook feed"),
    ("data.parse.hackernews-feed", "Parse Hacker News feed"),
    ("data.parse.linkedin-profile", "Parse LinkedIn profile"),
    ("data.parse.github-repo", "Parse GitHub repositories"),
];

static OBSERVATION_CACHE: Lazy<DashMap<String, ObservationEntry>> = Lazy::new(|| DashMap::new());
static QUOTE_ATTEMPT_CACHE: Lazy<DashMap<String, HashSet<String>>> = Lazy::new(|| DashMap::new());

#[derive(Clone, Debug)]
struct ObservationEntry {
    data: Value,
    parsed: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct KeyValueSelectorConfig {
    #[serde(default)]
    label: Option<String>,
    selector: Option<String>,
    #[serde(default)]
    attribute: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DomQuoteConfig {
    #[serde(default)]
    table_selectors: Vec<String>,
    #[serde(default)]
    key_value_selectors: Vec<KeyValueSelectorConfig>,
    #[serde(default)]
    max_rows: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct QuoteFetchPayload {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    table_selectors: Vec<String>,
    #[serde(default)]
    key_value_selectors: Vec<KeyValueSelectorConfig>,
    #[serde(default)]
    max_rows: Option<usize>,
    #[serde(default)]
    api: Option<ApiQuoteConfig>,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    market: Option<String>,
    #[serde(default)]
    fallback_sources: Vec<QuoteFallbackSource>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct QuoteFallbackSource {
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    market: Option<String>,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    table_selectors: Vec<String>,
    #[serde(default)]
    key_value_selectors: Vec<KeyValueSelectorConfig>,
    #[serde(default)]
    max_rows: Option<usize>,
    #[serde(default)]
    api: Option<ApiQuoteConfig>,
}

#[derive(Debug, Clone)]
struct QuoteAttemptPlan {
    source_id: Option<String>,
    market: Option<String>,
    source_url: Option<String>,
    table_selectors: Vec<String>,
    key_value_selectors: Vec<KeyValueSelectorConfig>,
    max_rows: Option<usize>,
    api: Option<ApiQuoteConfig>,
    prefer_api_first: bool,
}

impl QuoteAttemptPlan {
    fn from_primary(payload: &QuoteFetchPayload) -> Self {
        Self {
            source_id: payload
                .source_id
                .clone()
                .or_else(|| payload.source_url.clone()),
            market: payload.market.clone(),
            source_url: payload.source_url.clone(),
            table_selectors: if payload.table_selectors.is_empty() {
                vec!["table".to_string()]
            } else {
                payload.table_selectors.clone()
            },
            key_value_selectors: payload.key_value_selectors.clone(),
            max_rows: payload.max_rows.or(Some(50)),
            api: payload.api.clone(),
            prefer_api_first: matches!(payload.mode.as_deref(), Some("api") | Some("api_first")),
        }
    }

    fn from_fallback(source: &QuoteFallbackSource) -> Self {
        Self {
            source_id: source
                .source_id
                .clone()
                .or_else(|| source.source_url.clone()),
            market: source.market.clone(),
            source_url: source.source_url.clone(),
            table_selectors: if source.table_selectors.is_empty() {
                vec!["table".to_string()]
            } else {
                source.table_selectors.clone()
            },
            key_value_selectors: source.key_value_selectors.clone(),
            max_rows: source.max_rows.or(Some(50)),
            api: source.api.clone(),
            prefer_api_first: false,
        }
    }

    fn dom_attempt_key(&self) -> String {
        format!(
            "dom:{}:{}",
            self.source_id.as_deref().unwrap_or("unknown_dom_source"),
            self.source_url.as_deref().unwrap_or("")
        )
    }

    fn api_attempt_key(&self) -> String {
        format!(
            "api:{}:{}",
            self.source_id.as_deref().unwrap_or("unknown_api_source"),
            self.api.as_ref().map(|cfg| cfg.url.as_str()).unwrap_or("")
        )
    }

    fn should_try_dom_first(&self) -> bool {
        self.source_url.is_some() && !self.prefer_api_first
    }
}

fn build_quote_attempts(payload: &QuoteFetchPayload) -> Vec<QuoteAttemptPlan> {
    let mut attempts = Vec::new();
    attempts.push(QuoteAttemptPlan::from_primary(payload));
    for fallback in &payload.fallback_sources {
        attempts.push(QuoteAttemptPlan::from_fallback(fallback));
    }
    attempts
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MetalPriceValidationPayload {
    #[serde(default)]
    source_step_id: Option<String>,
    #[serde(default)]
    metal_keyword: Option<String>,
    #[serde(default)]
    allowed_markets: Vec<String>,
    #[serde(default = "default_validation_max_age")]
    max_age_hours: f64,
}

fn default_validation_max_age() -> f64 {
    24.0
}

#[derive(Debug, Clone, Deserialize, Default)]
struct TargetValidationPayload {
    #[serde(default)]
    source_step_id: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    allowed_domains: Vec<String>,
    #[serde(default = "default_expected_status")]
    expected_status: Option<u16>,
}

fn default_expected_status() -> Option<u16> {
    Some(200)
}

fn target_validation_error(code: &str, detail: &str) -> SoulBrowserError {
    let formatted = if detail.trim().is_empty() {
        format!("[{}] 目标页面校验失败", code)
    } else {
        format!("[{}] {}", code, detail)
    };
    SoulBrowserError::validation_error(&formatted, code)
}

fn mark_quote_attempt(subject_id: &str, attempt_key: &str) -> bool {
    if let Some(mut entry) = QUOTE_ATTEMPT_CACHE.get_mut(subject_id) {
        if entry.contains(attempt_key) {
            true
        } else {
            entry.insert(attempt_key.to_string());
            false
        }
    } else {
        let mut set = HashSet::new();
        set.insert(attempt_key.to_string());
        QUOTE_ATTEMPT_CACHE.insert(subject_id.to_string(), set);
        false
    }
}

fn reset_quote_attempts(subject_id: &str) {
    QUOTE_ATTEMPT_CACHE.remove(subject_id);
}

fn emit_self_heal_event(strategy_id: &str, note: Option<String>) {
    record_self_heal_event(SelfHealEvent {
        timestamp: Utc::now().timestamp_millis(),
        strategy_id: strategy_id.to_string(),
        action: "auto".to_string(),
        note,
    });
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ApiQuoteConfig {
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Option<Map<String, Value>>,
    #[serde(default)]
    headers: Option<HashMap<String, String>>,
    #[serde(default)]
    record_path: Option<Vec<String>>,
    #[serde(default)]
    field_mappings: Option<Vec<ApiFieldMapping>>,
    #[serde(default)]
    label_field: Option<String>,
    #[serde(default)]
    price_field: Option<String>,
    #[serde(default)]
    change_field: Option<String>,
    #[serde(default)]
    change_pct_field: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ApiFieldMapping {
    column: String,
    path: Vec<String>,
}

/// Browser tool manager using soulbase-tools
pub struct BrowserToolManager {
    registry: Arc<InMemoryRegistry>,
    tenant: TenantId,
    executor: Arc<dyn ToolExecutor>,
}

impl BrowserToolManager {
    /// Create new tool manager
    pub fn new(tenant_id: String) -> Self {
        Self {
            registry: Arc::new(InMemoryRegistry::new()),
            tenant: TenantId(tenant_id),
            executor: Arc::new(BrowserToolExecutor::new()),
        }
    }

    /// Return the current adapter mode when available
    pub fn adapter_mode(&self) -> Option<AdapterMode> {
        self.executor.adapter_mode()
    }

    pub fn cdp_adapter(&self) -> Option<Arc<CdpAdapter>> {
        self.executor.cdp_adapter()
    }

    /// Register a single tool by ID
    async fn register_tool(&self, id: &str) -> Result<(), SoulBrowserError> {
        let manifest = get_tool_manifest(id);
        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!("Failed to register tool '{}': {}", id, e))
            })
    }

    /// Register multiple tools by ID
    async fn register_tools(&self, ids: &[&str]) -> Result<(), SoulBrowserError> {
        for id in ids {
            self.register_tool(id).await?;
        }
        Ok(())
    }

    /// Register all default browser tools
    pub async fn register_default_tools(&self) -> Result<(), SoulBrowserError> {
        // Core browser tools from static config
        let core_tools: Vec<&str> = TOOL_CONFIGS.iter().map(|(id, _)| *id).collect();
        self.register_tools(&core_tools).await?;

        // Special tools
        self.register_tools(&[
            "take-screenshot",
            "manual.pointer",
            "data.extract-site",
            "market.quote.fetch",
            "data.deliver.structured",
        ])
        .await?;

        // Parse tools
        for (id, _) in DATA_PARSE_TOOLS {
            self.register_tool(id).await?;
        }

        // Legacy aliases
        self.register_tools(&[
            "browser.navigate",
            "browser.click",
            "browser.type",
            "browser.select",
            "browser.screenshot",
        ])
        .await?;

        Ok(())
    }

    // Convenience methods for individual tool registration (for backward compatibility)
    pub async fn register_navigation_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("navigate-to-url").await
    }
    pub async fn register_click_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("click").await
    }
    pub async fn register_type_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("type-text").await
    }
    pub async fn register_select_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("select-option").await
    }
    pub async fn register_scroll_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("scroll-page").await
    }
    pub async fn register_weather_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("weather.search").await
    }
    pub async fn register_browser_search_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("browser.search").await
    }
    pub async fn register_auto_act_click_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("browser.search.click-result").await
    }
    pub async fn register_close_modal_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("browser.close-modal").await
    }
    pub async fn register_send_escape_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("browser.send-esc").await
    }
    pub async fn register_wait_for_element_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("wait-for-element").await
    }
    pub async fn register_wait_for_condition_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("wait-for-condition").await
    }
    pub async fn register_get_element_info_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("get-element-info").await
    }
    pub async fn register_retrieve_history_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("retrieve-history").await
    }
    pub async fn register_complete_task_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("complete-task").await
    }
    pub async fn register_report_insight_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("report-insight").await
    }
    pub async fn register_observation_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("data.extract-site").await
    }
    pub async fn register_take_screenshot_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("take-screenshot").await
    }
    pub async fn register_pointer_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("manual.pointer").await
    }
    pub async fn register_quote_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("market.quote.fetch").await
    }
    pub async fn register_deliver_tool(&self) -> Result<(), SoulBrowserError> {
        self.register_tool("data.deliver.structured").await
    }
    pub async fn register_data_parse_tools(&self) -> Result<(), SoulBrowserError> {
        for (id, _) in DATA_PARSE_TOOLS {
            self.register_tool(id).await?;
        }
        Ok(())
    }
    pub async fn register_legacy_aliases(&self) -> Result<(), SoulBrowserError> {
        self.register_tools(&[
            "browser.navigate",
            "browser.click",
            "browser.type",
            "browser.select",
            "browser.screenshot",
        ])
        .await
    }

    /// List available tools
    pub async fn list_tools(
        &self,
        filter: Option<String>,
    ) -> Result<Vec<AvailableSpec>, SoulBrowserError> {
        let list_filter = ListFilter {
            tag: filter,
            include_disabled: false,
        };

        self.registry
            .list(&self.tenant, &list_filter)
            .await
            .map_err(|e| SoulBrowserError::internal(&format!("Failed to list tools: {}", e)))
    }

    /// Get tool by ID
    pub async fn get_tool(&self, tool_id: &str) -> Result<Option<AvailableSpec>, SoulBrowserError> {
        let id = ToolId(tool_id.to_string());

        self.registry
            .get(&self.tenant, &id)
            .await
            .map_err(|e| SoulBrowserError::internal(&format!("Failed to get tool: {}", e)))
    }

    /// Execute a tool by ID
    pub async fn execute(
        &self,
        tool_id: &str,
        subject_id: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, SoulBrowserError> {
        self.execute_with_route(tool_id, subject_id, input, None, None)
            .await
    }

    /// Execute a tool by ID with an explicit route
    pub async fn execute_with_route(
        &self,
        tool_id: &str,
        subject_id: &str,
        input: serde_json::Value,
        route: Option<ExecRoute>,
        timeout_ms: Option<u64>,
    ) -> Result<serde_json::Value, SoulBrowserError> {
        // Check if tool exists
        let id = ToolId(tool_id.to_string());
        let tool = self
            .registry
            .get(&self.tenant, &id)
            .await
            .map_err(|e| SoulBrowserError::internal(&format!("Tool not found: {}", e)))?;

        if tool.is_none() {
            return Err(SoulBrowserError::not_found(&format!(
                "Tool not found: {}",
                tool_id
            )));
        }

        // Create execution context
        let effective_timeout = timeout_ms.unwrap_or(DEFAULT_TOOL_TIMEOUT_MS);
        let context = ToolExecutionContext {
            tool_id: tool_id.to_string(),
            tenant_id: self.tenant.0.clone(),
            subject_id: subject_id.to_string(),
            input,
            timeout_ms: effective_timeout,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route,
        };

        let result = self.executor.execute(context).await?;
        Ok(serde_json::to_value(result)?)
    }
}

/// Tool configuration for declarative manifest creation
struct ToolConfig {
    display_name: &'static str,
    description: &'static str,
    tags: &'static [&'static str],
    scope: &'static str,
    domain: &'static str,
    action: &'static str,
    side_effect: SideEffect,
    safety_class: SafetyClass,
    timeout_ms: u64,
    max_bytes_in: u64,
    max_bytes_out: u64,
    max_files: u64,
    max_concurrency: u32,
    idempotency: IdempoKind,
    concurrency: ConcurrencyKind,
}

impl ToolConfig {
    const fn browser(
        display_name: &'static str,
        description: &'static str,
        tags: &'static [&'static str],
        scope: &'static str,
        action: &'static str,
        timeout_ms: u64,
        max_bytes_in: u64,
        max_bytes_out: u64,
    ) -> Self {
        Self {
            display_name,
            description,
            tags,
            scope,
            domain: "browser",
            action,
            side_effect: SideEffect::Browser,
            safety_class: SafetyClass::Low,
            timeout_ms,
            max_bytes_in,
            max_bytes_out,
            max_files: 0,
            max_concurrency: 1,
            idempotency: IdempoKind::None,
            concurrency: ConcurrencyKind::Queue,
        }
    }

    const fn read_only(
        display_name: &'static str,
        description: &'static str,
        tags: &'static [&'static str],
        scope: &'static str,
        domain: &'static str,
        action: &'static str,
        timeout_ms: u64,
        max_bytes_in: u64,
        max_bytes_out: u64,
        max_concurrency: u32,
    ) -> Self {
        Self {
            display_name,
            description,
            tags,
            scope,
            domain,
            action,
            side_effect: SideEffect::Read,
            safety_class: SafetyClass::Low,
            timeout_ms,
            max_bytes_in,
            max_bytes_out,
            max_files: 0,
            max_concurrency,
            idempotency: IdempoKind::None,
            concurrency: ConcurrencyKind::Queue,
        }
    }
}

/// Static tool configurations - replaces 20+ individual create_xxx_tool functions
static TOOL_CONFIGS: &[(&str, ToolConfig)] = &[
    (
        "navigate-to-url",
        ToolConfig::browser(
            "Navigate to URL",
            "Navigate browser to specified URL",
            &["browser", "navigation"],
            "browser:navigate",
            "navigate",
            30_000,
            1024 * 1024,
            10 * 1024 * 1024,
        ),
    ),
    (
        "click",
        ToolConfig::browser(
            "Click Element",
            "Click on a page element using selector",
            &["browser", "interaction"],
            "browser:interact",
            "click",
            10_000,
            1024,
            1024,
        ),
    ),
    (
        "type-text",
        ToolConfig::browser(
            "Type Text",
            "Type text into an input element",
            &["browser", "input"],
            "browser:interact",
            "type",
            10_000,
            10 * 1024,
            1024,
        ),
    ),
    (
        "select-option",
        ToolConfig::browser(
            "Select Option",
            "Choose an option from a select control",
            &["browser", "interaction"],
            "browser:interact",
            "select",
            10_000,
            2048,
            1024,
        ),
    ),
    (
        "scroll-page",
        ToolConfig::browser(
            "Scroll Page",
            "Scroll the page or a container to reveal targets",
            &["browser", "navigation"],
            "browser:interact",
            "scroll",
            5_000,
            8 * 1024,
            2 * 1024,
        ),
    ),
    (
        "weather.search",
        ToolConfig::browser(
            "Weather Search",
            "Navigate to a weather search page and ensure the widget loads",
            &["browser", "macro", "weather"],
            "browser:macro",
            "weather-search",
            45_000,
            8 * 1024,
            8 * 1024,
        ),
    ),
    (
        "browser.search",
        ToolConfig::browser(
            "Web Search",
            "Open a search engine results page for the given query",
            &["browser", "macro", "search"],
            "browser:macro",
            "search",
            30_000,
            8 * 1024,
            8 * 1024,
        ),
    ),
    (
        "browser.search.click-result",
        ToolConfig::browser(
            "Search Result Click",
            "Scan SERP DOM for authority domains and click the best match.",
            &["browser", "macro"],
            "browser:macro",
            "macro",
            20_000,
            8 * 1024,
            8 * 1024,
        ),
    ),
    (
        "browser.close-modal",
        ToolConfig::browser(
            "Close Modal",
            "Attempt to dismiss popup dialogs by clicking close controls or sending ESC",
            &["browser", "interaction"],
            "browser:interact",
            "dismiss",
            8_000,
            4 * 1024,
            4 * 1024,
        ),
    ),
    (
        "browser.send-esc",
        ToolConfig::browser(
            "Send Escape",
            "Dispatch Escape key events to the active page",
            &["browser", "macro"],
            "browser:macro",
            "keyboard",
            5_000,
            2 * 1024,
            2 * 1024,
        ),
    ),
    (
        "wait-for-element",
        ToolConfig::read_only(
            "Wait For Element",
            "Wait until a structural element condition is met",
            &["browser", "synchronization"],
            "browser:wait",
            "browser",
            "wait-element",
            60_000,
            16 * 1024,
            4 * 1024,
            4,
        ),
    ),
    (
        "wait-for-condition",
        ToolConfig::read_only(
            "Wait For Condition",
            "Wait until network/runtime conditions meet expectations",
            &["browser", "synchronization"],
            "browser:wait",
            "browser",
            "wait-condition",
            60_000,
            16 * 1024,
            4 * 1024,
            4,
        ),
    ),
    (
        "get-element-info",
        ToolConfig::read_only(
            "Get Element Info",
            "Collect structural details about an element",
            &["browser", "memory"],
            "browser:inspect",
            "browser",
            "inspect",
            5_000,
            32 * 1024,
            32 * 1024,
            2,
        ),
    ),
    (
        "retrieve-history",
        ToolConfig::read_only(
            "Retrieve History",
            "Retrieve recent tool execution or perception history",
            &["browser", "memory"],
            "browser:history",
            "browser",
            "history",
            3_000,
            8 * 1024,
            64 * 1024,
            2,
        ),
    ),
    (
        "complete-task",
        ToolConfig::read_only(
            "Complete Task",
            "Record task completion metadata and evidence",
            &["metacognition", "task"],
            "task:complete",
            "task",
            "complete",
            2_000,
            32 * 1024,
            16 * 1024,
            1,
        ),
    ),
    (
        "report-insight",
        ToolConfig::read_only(
            "Report Insight",
            "Record insights or observations for downstream consumers",
            &["metacognition", "insight"],
            "task:insight",
            "task",
            "insight",
            2_000,
            24 * 1024,
            16 * 1024,
            4,
        ),
    ),
];

/// Create tool manifest from configuration
fn create_tool_from_config(id: &str, config: &ToolConfig) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: config.display_name.to_string(),
        description: config.description.to_string(),
        tags: config.tags.iter().map(|s| s.to_string()).collect(),
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec![config.scope.to_string()],
        capabilities: vec![CapabilityDecl {
            domain: config.domain.to_string(),
            action: config.action.to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: config.side_effect.clone(),
        safety_class: config.safety_class.clone(),
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: config.timeout_ms,
            max_bytes_in: config.max_bytes_in,
            max_bytes_out: config.max_bytes_out,
            max_files: config.max_files,
            max_depth: 0,
            max_concurrency: config.max_concurrency,
        },
        idempotency: config.idempotency.clone(),
        concurrency: config.concurrency.clone(),
    }
}

/// Get tool manifest by ID from static config or special cases
fn get_tool_manifest(id: &str) -> ToolManifest {
    // Check static configs first
    if let Some((_, config)) = TOOL_CONFIGS.iter().find(|(tid, _)| *tid == id) {
        return create_tool_from_config(id, config);
    }
    // Special cases with unique configurations
    match id {
        "market.quote.fetch" => create_quote_fetch_tool(id),
        "take-screenshot" | "browser.screenshot" => create_take_screenshot_tool(id),
        "manual.pointer" => create_pointer_tool(id),
        "data.extract-site" => create_observation_tool(id),
        _ if id.starts_with("data.parse.") || id.starts_with("data.validate") => {
            create_parse_tool(id)
        }
        "data.deliver.structured" => create_deliver_tool(id),
        // Legacy aliases
        "browser.navigate" => get_tool_manifest("navigate-to-url"),
        "browser.click" => get_tool_manifest("click"),
        "browser.type" => get_tool_manifest("type-text"),
        "browser.select" => get_tool_manifest("select-option"),
        _ => create_tool_from_config(id, &TOOL_CONFIGS[0].1), // fallback
    }
}

fn create_quote_fetch_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Fetch Metal Quotes".to_string(),
        description: "Capture metal quote tables via DOM or API".to_string(),
        tags: vec![
            "browser".to_string(),
            "quote".to_string(),
            "observation".to_string(),
        ],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:observe".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "quote-fetch".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 20_000,
            max_bytes_in: 32 * 1024,
            max_bytes_out: 64 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 2,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Queue,
    }
}

fn create_take_screenshot_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Take Screenshot".to_string(),
        description: "Capture screenshot of current page".to_string(),
        tags: vec!["browser".to_string(), "capture".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:read".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "screenshot".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 5000,
            max_bytes_in: 1024,
            max_bytes_out: 50 * 1024 * 1024,
            max_files: 1,
            max_depth: 0,
            max_concurrency: 1,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Parallel,
    }
}

fn create_pointer_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Manual Pointer Event".to_string(),
        description: "Inject low-level pointer interactions for live control".to_string(),
        tags: vec!["browser".to_string(), "pointer".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:interact".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "pointer".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Browser,
        safety_class: SafetyClass::Medium,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 2000,
            max_bytes_in: 2048,
            max_bytes_out: 2048,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 2,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Parallel,
    }
}

fn create_observation_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Capture Page Observation".to_string(),
        description: "Capture DOM/text snapshot for downstream parsing".to_string(),
        tags: vec!["data".to_string(), "observe".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:read".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "observe".to_string(),
            resource: "*".to_string(),
            attrs: Value::Null,
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 30_000,
            max_bytes_in: 4 * 1024,
            max_bytes_out: 2 * 1024 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 1,
        },
        idempotency: IdempoKind::Keyed,
        concurrency: ConcurrencyKind::Queue,
    }
}

fn create_parse_tool(id: &str) -> ToolManifest {
    let description = DATA_PARSE_TOOLS
        .iter()
        .find(|(tool_id, _)| *tool_id == id)
        .map(|(_, desc)| *desc)
        .unwrap_or("Parse structured observation data");

    let (display_name, capability_action, tags) = if id.starts_with("data.validate.") {
        (
            format!("Validate {}", id.trim_start_matches("data.validate.")),
            "validate".to_string(),
            vec!["data".to_string(), "validate".to_string()],
        )
    } else {
        (
            format!("{}", id.replace("data.parse.", "Parse ")),
            "parse".to_string(),
            vec!["data".to_string(), "parse".to_string()],
        )
    };

    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name,
        description: description.to_string(),
        tags,
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["data:read".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "data".to_string(),
            action: capability_action,
            resource: id.to_string(),
            attrs: Value::Null,
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 15_000,
            max_bytes_in: 2 * 1024 * 1024,
            max_bytes_out: 512 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 2,
        },
        idempotency: IdempoKind::Keyed,
        concurrency: ConcurrencyKind::Parallel,
    }
}

fn create_deliver_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Deliver Structured Artifact".to_string(),
        description: "Persist structured data to the artifacts directory".to_string(),
        tags: vec!["data".to_string(), "deliver".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["data:write".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "data".to_string(),
            action: "deliver".to_string(),
            resource: "structured".to_string(),
            attrs: Value::Null,
        }],
        side_effect: SideEffect::Write,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 20_000,
            max_bytes_in: 2 * 1024 * 1024,
            max_bytes_out: 64 * 1024,
            max_files: 2,
            max_depth: 0,
            max_concurrency: 1,
        },
        idempotency: IdempoKind::Keyed,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create a simple schema for tool manifests
fn create_simple_schema() -> RootSchema {
    let mut schema = RootSchema::default();
    schema.schema = SchemaObject::default().into();
    schema
}

/// Tool execution context
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolExecutionContext {
    pub tool_id: String,
    pub tenant_id: String,
    pub subject_id: String,
    pub input: serde_json::Value,
    pub timeout_ms: u64,
    pub trace_id: String,
    pub route: Option<ExecRoute>,
}

/// Tool execution result
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolExecutionResult {
    pub success: bool,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

/// Tool executor trait
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a tool
    async fn execute(
        &self,
        context: ToolExecutionContext,
    ) -> Result<ToolExecutionResult, SoulBrowserError>;

    /// Access the underlying CDP adapter when available
    fn cdp_adapter(&self) -> Option<Arc<CdpAdapter>> {
        None
    }

    /// Report the adapter mode when available (real vs stub)
    fn adapter_mode(&self) -> Option<AdapterMode> {
        self.cdp_adapter().as_ref().map(|adapter| adapter.mode())
    }
}

/// Browser tool executor implementation
pub struct BrowserToolExecutor {
    adapter: Arc<CdpAdapter>,
    primitives: Arc<DefaultActionPrimitives>,
    policy_view: Arc<PolicyView>,
    adapter_ready: OnceCell<()>,
    route_pages: DashMap<String, cdp_adapter::PageId>,
    page_routes: DashMap<cdp_adapter::PageId, String>,
    pending_pages: DashMap<String, Instant>,
}

impl BrowserToolExecutor {
    pub fn new() -> Self {
        obs_tracing::init_tracing();
        let (event_bus, _rx) = event_bus(64);
        let adapter = Arc::new(CdpAdapter::new(CdpConfig::default(), event_bus));
        let wait_strategy = Arc::new(DefaultWaitStrategy::default());
        let primitives = Arc::new(DefaultActionPrimitives::new(adapter.clone(), wait_strategy));
        let policy_view = Arc::new(PolicyView::from(default_snapshot()));

        Self {
            adapter,
            primitives,
            policy_view,
            adapter_ready: OnceCell::new(),
            route_pages: DashMap::new(),
            page_routes: DashMap::new(),
            pending_pages: DashMap::new(),
        }
    }

    async fn ensure_adapter_started(&self) -> Result<(), SoulBrowserError> {
        self.adapter_ready
            .get_or_try_init(|| async {
                Arc::clone(&self.adapter).start().await.map_err(|err| {
                    SoulBrowserError::internal(&format!("Failed to start CDP adapter: {}", err))
                })
            })
            .await
            .map(|_| ())
    }

    async fn dispatch_keyboard_event(
        &self,
        exec_ctx: &ExecCtx,
        key: &str,
        code: &str,
        key_code: u32,
        focus_selector: Option<&str>,
    ) -> Result<bool, SoulBrowserError> {
        let context = self
            .primitives
            .resolve_context(exec_ctx)
            .await
            .map_err(|err| {
                SoulBrowserError::internal(&format!("Failed to resolve execution context: {}", err))
            })?;
        let script = keyboard_event_script(key, code, key_code, focus_selector);
        let value = self
            .primitives
            .adapter()
            .evaluate_script_in_context(&context, &script)
            .await
            .map_err(|err| {
                SoulBrowserError::internal(&format!("Failed to dispatch {key} event: {}", err))
            })?;
        Ok(value.as_bool().unwrap_or(false))
    }

    fn finish_tool(
        &self,
        context: &ToolExecutionContext,
        start: Instant,
        span: &tracing::Span,
        result: ToolExecutionResult,
    ) -> ToolExecutionResult {
        let duration_ms = start.elapsed().as_millis() as u64;
        obs_tracing::observe_latency(span, duration_ms);

        let mut labels: ObsLabelMap = ObsLabelMap::new();
        labels.insert("tool".into(), context.tool_id.clone());
        labels.insert("success".into(), result.success.to_string());
        obs_metrics::inc(METRIC_TOOL_INVOCATIONS, labels.clone());
        obs_metrics::observe(METRIC_TOOL_LATENCY, duration_ms, labels);

        result
    }

    fn record_error(&self, context: &ToolExecutionContext, start: Instant, span: &tracing::Span) {
        let duration_ms = start.elapsed().as_millis() as u64;
        obs_tracing::observe_latency(span, duration_ms);

        let mut labels: ObsLabelMap = ObsLabelMap::new();
        labels.insert("tool".into(), context.tool_id.clone());
        labels.insert("success".into(), "false".into());
        obs_metrics::inc(METRIC_TOOL_INVOCATIONS, labels.clone());
        obs_metrics::observe(METRIC_TOOL_LATENCY, duration_ms, labels);
    }

    fn map_action_error(action: &str, err: ActionError) -> SoulBrowserError {
        match err {
            ActionError::AnchorNotFound(detail) => {
                let user_msg = format!("{action} target not found: {detail}");
                let dev_msg = format!("{action} failed: {detail}");
                SoulBrowserError::validation_error(&user_msg, &dev_msg)
            }
            ActionError::NotClickable(detail) => {
                let user_msg = format!("{action} target not clickable: {detail}");
                SoulBrowserError::validation_error(&user_msg, &user_msg)
            }
            ActionError::NotEnabled(detail) => {
                let user_msg = format!("{action} target not enabled: {detail}");
                SoulBrowserError::validation_error(&user_msg, &user_msg)
            }
            ActionError::OptionNotFound(detail) | ActionError::ScrollTargetInvalid(detail) => {
                let user_msg = format!("{action} failed: {detail}");
                SoulBrowserError::validation_error(&user_msg, &user_msg)
            }
            ActionError::PolicyDenied(detail) => {
                let user_msg = format!("{action} not allowed: {detail}");
                SoulBrowserError::forbidden(&user_msg)
            }
            other => SoulBrowserError::internal(&format!("{action} failed: {}", other)),
        }
    }

    async fn resolve_page_for_route(
        &self,
        route: &ExecRoute,
    ) -> Result<cdp_adapter::PageId, SoulBrowserError> {
        let route_key = route.page.0.clone();

        if let Some(existing) = self.route_pages.get(&route_key) {
            return Ok(*existing.value());
        }

        self.ensure_adapter_started().await?;

        let deadline = Instant::now() + Duration::from_secs(5);

        loop {
            if Instant::now() >= deadline {
                return Err(SoulBrowserError::internal(
                    "No available CDP pages for execution route",
                ));
            }

            self.cleanup_stale_mappings();

            if let Some(existing) = self.route_pages.get(&route_key) {
                return Ok(*existing.value());
            }

            let claimed: HashSet<cdp_adapter::PageId> =
                self.page_routes.iter().map(|entry| *entry.key()).collect();

            let candidate = self
                .adapter
                .registry()
                .iter()
                .into_iter()
                .filter(|(_, ctx)| ctx.cdp_session.is_some())
                .map(|(page, _)| page)
                .find(|page| !claimed.contains(page));

            if let Some(page) = candidate {
                self.route_pages.insert(route_key.clone(), page);
                self.page_routes.insert(page, route_key.clone());
                return Ok(page);
            } else {
                if self.pending_pages.get(&route_key).is_some() {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }

                self.pending_pages.insert(route_key.clone(), Instant::now());

                let create_result = self.adapter.create_page("about:blank").await;
                self.pending_pages.remove(&route_key);

                match create_result {
                    Ok(_) => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                    Err(err) => {
                        if self.adapter.registry().iter().is_empty() {
                            let synthetic = Self::synthetic_page_id(route);
                            self.route_pages.insert(route_key.clone(), synthetic);
                            return Ok(synthetic);
                        }

                        return Err(SoulBrowserError::internal(&format!(
                            "Failed to create CDP page: {}",
                            err
                        )));
                    }
                }
            }
        }
    }

    fn cleanup_stale_mappings(&self) {
        let active_pages: HashSet<cdp_adapter::PageId> = self
            .adapter
            .registry()
            .iter()
            .into_iter()
            .map(|(page, _)| page)
            .collect();

        if active_pages.is_empty() {
            return;
        }

        self.route_pages
            .retain(|_, page| active_pages.contains(page));
        self.page_routes
            .retain(|page, _| active_pages.contains(page));
    }

    fn synthetic_page_id(route: &ExecRoute) -> cdp_adapter::PageId {
        match Uuid::parse_str(&route.page.0) {
            Ok(id) => cdp_adapter::PageId(id),
            Err(_) => cdp_adapter::PageId(Uuid::new_v4()),
        }
    }

    fn build_exec_ctx(&self, route: Option<&ExecRoute>, timeout_ms: u64) -> ExecCtx {
        let route = route.cloned().unwrap_or_else(|| {
            let session = SessionId::new();
            let page = PageId::new();
            let frame = FrameId::new();
            ExecRoute::new(session, page, frame)
        });
        let timeout = if timeout_ms == 0 { 30_000 } else { timeout_ms };
        let deadline = Instant::now() + Duration::from_millis(timeout);

        ExecCtx::new(
            route,
            deadline,
            CancellationToken::new(),
            (*self.policy_view).clone(),
        )
    }

    fn parse_wait_tier(value: Option<&serde_json::Value>) -> WaitTier {
        match value
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase())
        {
            Some(ref tier) if tier == "none" => WaitTier::None,
            Some(ref tier) if tier == "idle" => WaitTier::Idle,
            Some(ref tier) if tier == "domready" => WaitTier::DomReady,
            _ => WaitTier::DomReady,
        }
    }

    fn parse_select_method(value: Option<&str>) -> Result<SelectMethod, SoulBrowserError> {
        match value.map(|v| v.to_ascii_lowercase()) {
            None => Ok(SelectMethod::default()),
            Some(kind) if kind == "value" => Ok(SelectMethod::Value),
            Some(kind) if kind == "text" || kind == "label" => Ok(SelectMethod::Text),
            Some(kind) if kind == "index" => Ok(SelectMethod::Index),
            Some(other) => Err(SoulBrowserError::validation_error(
                "Invalid match kind",
                &format!(
                    "Unsupported select match kind '{}'. Expected value, text, or index.",
                    other
                ),
            )),
        }
    }

    async fn execute_observation_tool(
        &self,
        context: &ToolExecutionContext,
        start: Instant,
        span: &tracing::Span,
        exec_ctx: ExecCtx,
    ) -> Result<ToolExecutionResult, SoulBrowserError> {
        self.ensure_adapter_started().await?;
        let page_id = self.resolve_page_for_route(&exec_ctx.route).await?;
        let raw = self
            .adapter
            .evaluate_script(page_id, PAGE_OBSERVE_SCRIPT)
            .await
            .map_err(|err| {
                self.record_error(context, start, span);
                SoulBrowserError::internal(&format!("Observation script failed: {}", err))
            })?;

        let ok = raw.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            let reason = raw
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            self.record_error(context, start, span);
            return Err(SoulBrowserError::internal(&format!(
                "Observation script reported failure: {}",
                reason
            )));
        }

        let data_value = raw.get("data").cloned().unwrap_or(Value::Null);
        OBSERVATION_CACHE.insert(
            context.subject_id.clone(),
            ObservationEntry {
                data: data_value.clone(),
                parsed: HashMap::new(),
            },
        );

        let mut metadata = serde_json::Map::new();
        metadata.insert("subject_id".to_string(), json!(context.subject_id.clone()));
        let output = ToolExecutionResult {
            success: true,
            output: Some(json!({
                "status": "captured",
                "observation": data_value,
                "preview": raw.get("preview").cloned(),
            })),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
            metadata,
        };
        Ok(self.finish_tool(context, start, span, output))
    }

    async fn execute_quote_fetch_tool(
        &self,
        context: &ToolExecutionContext,
        start: Instant,
        span: &tracing::Span,
        exec_ctx: ExecCtx,
    ) -> Result<ToolExecutionResult, SoulBrowserError> {
        let payload_value = context
            .input
            .get("payload")
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let payload: QuoteFetchPayload = serde_json::from_value(payload_value).map_err(|err| {
            self.record_error(context, start, span);
            SoulBrowserError::validation_error("invalid_payload", &err.to_string())
        })?;

        let mut observation: Option<Value> = None;
        let mut last_error: Option<SoulBrowserError> = None;
        let mut attempt_log: Vec<serde_json::Value> = Vec::new();
        let subject_id = context.subject_id.clone();
        let attempts = build_quote_attempts(&payload);
        let mut dom_block_reason_meta: Option<String> = None;
        let mut dom_blocked_url: Option<String> = None;
        let mut winning_source: Option<String> = None;
        let mut winning_market: Option<String> = None;
        let total_attempts = attempts.len();

        for (idx, attempt) in attempts.into_iter().enumerate() {
            let has_more = idx + 1 < total_attempts;
            let mut dom_block_reason_current: Option<String> = None;
            let mut dom_failed_for_attempt = false;

            if attempt.prefer_api_first {
                if let Some(api_cfg) = attempt.api.as_ref() {
                    let api_key = attempt.api_attempt_key();
                    if mark_quote_attempt(&subject_id, &api_key) {
                        attempt_log.push(json!({
                            "mode": "api",
                            "status": "skipped",
                            "reason": "already_attempted",
                            "source": attempt.source_id,
                        }));
                    } else {
                        match self
                            .fetch_quotes_via_api(
                                api_cfg,
                                attempt.source_url.clone(),
                                attempt.max_rows,
                            )
                            .await
                        {
                            Ok(value) => {
                                record_market_quote_fetch("api", "success");
                                attempt_log.push(json!({
                                    "mode": "api",
                                    "status": "success",
                                    "source": attempt.source_id,
                                }));
                                winning_source = attempt.source_id.clone();
                                winning_market = attempt.market.clone();
                                observation = Some(value);
                                break;
                            }
                            Err(err) => {
                                record_market_quote_fetch("api", "failed");
                                attempt_log.push(json!({
                                    "mode": "api",
                                    "status": "failed",
                                    "source": attempt.source_id,
                                }));
                                last_error = Some(err);
                            }
                        }
                    }
                }
            }

            if observation.is_some() {
                break;
            }

            if attempt.source_url.is_some() {
                let dom_key = attempt.dom_attempt_key();
                if mark_quote_attempt(&subject_id, &dom_key) {
                    attempt_log.push(json!({
                        "mode": "dom",
                        "status": "skipped",
                        "reason": "already_attempted",
                        "source": attempt.source_id,
                    }));
                } else {
                    match self.fetch_quotes_via_dom(&attempt, exec_ctx.clone()).await {
                        Ok(value) => {
                            let block_notice = detect_block_reason(
                                "",
                                value
                                    .get("text_sample")
                                    .and_then(Value::as_str)
                                    .unwrap_or(""),
                                value.get("url").and_then(Value::as_str),
                            );
                            if let Some(reason) = block_notice {
                                record_market_quote_fetch("dom", "blocked");
                                attempt_log.push(json!({
                                    "mode": "dom",
                                    "status": "blocked",
                                    "source": attempt.source_id,
                                    "reason": reason,
                                }));
                                dom_block_reason_current = Some(reason.clone());
                                dom_block_reason_meta = Some(reason.clone());
                                dom_blocked_url = value
                                    .get("url")
                                    .and_then(Value::as_str)
                                    .map(|s| s.to_string());
                                if let Some(source) = attempt.source_id.as_deref() {
                                    mark_source_unhealthy(source, &reason);
                                }
                                emit_self_heal_event("manual_takeover_hint", Some(reason.clone()));
                            } else {
                                record_market_quote_fetch("dom", "success");
                                attempt_log.push(json!({
                                    "mode": "dom",
                                    "status": "success",
                                    "source": attempt.source_id,
                                }));
                                winning_source = attempt.source_id.clone();
                                winning_market = attempt.market.clone();
                                observation = Some(value);
                                break;
                            }
                        }
                        Err(err) => {
                            record_market_quote_fetch("dom", "failed");
                            attempt_log.push(json!({
                                "mode": "dom",
                                "status": "failed",
                                "source": attempt.source_id,
                            }));
                            last_error = Some(err);
                            dom_failed_for_attempt = true;
                        }
                    }
                }
            }

            if observation.is_some() {
                break;
            }

            if dom_block_reason_current.is_some()
                && attempt.api.is_some()
                && !attempt.prefer_api_first
            {
                record_market_quote_fallback("dom_soft_block");
            }

            if (!attempt.prefer_api_first && attempt.api.is_some())
                && (dom_failed_for_attempt
                    || dom_block_reason_current.is_some()
                    || attempt.source_url.is_none())
            {
                if dom_failed_for_attempt && attempt.source_url.is_some() {
                    record_market_quote_fallback("dom_to_api");
                }
                let api_key = attempt.api_attempt_key();
                if mark_quote_attempt(&subject_id, &api_key) {
                    attempt_log.push(json!({
                        "mode": "api",
                        "status": "skipped",
                        "reason": "already_attempted",
                        "source": attempt.source_id,
                    }));
                } else if let Some(api_cfg) = attempt.api.as_ref() {
                    match self
                        .fetch_quotes_via_api(api_cfg, attempt.source_url.clone(), attempt.max_rows)
                        .await
                    {
                        Ok(value) => {
                            record_market_quote_fetch("api", "success");
                            attempt_log.push(json!({
                                "mode": "api",
                                "status": "success",
                                "source": attempt.source_id,
                            }));
                            winning_source = attempt.source_id.clone();
                            winning_market = attempt.market.clone();
                            observation = Some(value);
                            break;
                        }
                        Err(err) => {
                            record_market_quote_fetch("api", "failed");
                            attempt_log.push(json!({
                                "mode": "api",
                                "status": "failed",
                                "source": attempt.source_id,
                            }));
                            last_error = Some(err);
                        }
                    }
                }
            }

            if observation.is_some() {
                break;
            }

            if has_more {
                record_market_quote_fallback("rotate_source");
            }
        }

        let data = match observation {
            Some(value) => value,
            None => {
                let err = last_error.unwrap_or_else(|| {
                    SoulBrowserError::validation_error(
                        "报价采集失败：所有数据源均不可用",
                        "quote fetch failed: no mode succeeded",
                    )
                });
                self.record_error(context, start, span);
                return Err(err);
            }
        };

        let block_notice = detect_block_reason(
            "",
            data.get("text_sample")
                .and_then(Value::as_str)
                .unwrap_or(""),
            data.get("url").and_then(Value::as_str),
        );
        if let Some(reason) = &block_notice {
            emit_self_heal_event("manual_takeover_hint", Some(reason.clone()));
        }

        OBSERVATION_CACHE.insert(
            context.subject_id.clone(),
            ObservationEntry {
                data: data.clone(),
                parsed: HashMap::new(),
            },
        );
        reset_quote_attempts(&subject_id);

        let mut metadata = serde_json::Map::new();
        metadata.insert("subject_id".to_string(), json!(context.subject_id.clone()));
        if !attempt_log.is_empty() {
            metadata.insert("attempts".to_string(), Value::Array(attempt_log));
        }
        if let Some(source_id) = winning_source {
            metadata.insert("source_id".to_string(), json!(source_id));
        }
        if let Some(market) = winning_market {
            metadata.insert("source_market".to_string(), json!(market));
        }
        if let Some(reason) = dom_block_reason_meta {
            metadata.insert("dom_block_reason".to_string(), json!(reason));
        }
        if let Some(url) = dom_blocked_url {
            metadata.insert("dom_blocked_url".to_string(), json!(url));
        }
        if let Some(reason) = block_notice {
            metadata.insert("block_reason".to_string(), json!(reason));
        }
        let output = ToolExecutionResult {
            success: true,
            output: Some(json!({
                "status": "captured",
                "observation": data,
            })),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
            metadata,
        };
        Ok(self.finish_tool(context, start, span, output))
    }

    async fn fetch_quotes_via_dom(
        &self,
        attempt: &QuoteAttemptPlan,
        exec_ctx: ExecCtx,
    ) -> Result<Value, SoulBrowserError> {
        self.ensure_adapter_started().await?;
        let page_id = self.resolve_page_for_route(&exec_ctx.route).await?;
        let dom_config = DomQuoteConfig {
            table_selectors: if attempt.table_selectors.is_empty() {
                vec!["table".to_string()]
            } else {
                attempt.table_selectors.clone()
            },
            key_value_selectors: attempt.key_value_selectors.clone(),
            max_rows: attempt.max_rows,
        };
        let config_json = serde_json::to_string(&dom_config).map_err(|err| {
            SoulBrowserError::internal(&format!("Quote config serialization failed: {err}"))
        })?;
        let expression = QUOTE_FETCH_SCRIPT.replace("__CONFIG__", &config_json);
        let raw = self
            .adapter
            .evaluate_script(page_id, &expression)
            .await
            .map_err(|err| {
                SoulBrowserError::internal(&format!("Quote fetch script failed: {err}"))
            })?;
        let ok = raw.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            return Err(SoulBrowserError::internal(
                raw.get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Quote fetch script reported failure"),
            ));
        }
        Ok(raw.get("data").cloned().unwrap_or(Value::Null))
    }

    async fn fetch_quotes_via_api(
        &self,
        api: &ApiQuoteConfig,
        source_url: Option<String>,
        max_rows: Option<usize>,
    ) -> Result<Value, SoulBrowserError> {
        let client = Client::new();
        let method = api.method.as_deref().unwrap_or("GET").to_ascii_uppercase();
        let mut request = match method.as_str() {
            "POST" => client.post(&api.url),
            _ => client.get(&api.url),
        };
        if let Some(headers) = &api.headers {
            for (key, value) in headers.iter() {
                request = request.header(key, value);
            }
        }
        if let Some(params) = &api.params {
            if method == "POST" {
                request = request.json(params);
            } else {
                let query: Vec<(String, String)> = params
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::format_api_value(v)))
                    .collect();
                request = request.query(&query);
            }
        }
        let response = request.send().await.map_err(|err| {
            SoulBrowserError::internal(&format!("Quote API request failed: {err}"))
        })?;
        let status = response.status();
        let body = response.text().await.map_err(|err| {
            SoulBrowserError::internal(&format!("Quote API body read failed: {err}"))
        })?;
        if !status.is_success() {
            return Err(SoulBrowserError::internal(&format!(
                "Quote API returned {}",
                status
            )));
        }
        let json_value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let (tables, key_values) = Self::convert_api_response(&json_value, api, max_rows);
        let text_sample = body.chars().take(2000).collect::<String>();
        Ok(json!({
            "kind": "quote_observation",
            "source": "market.quote.fetch.api",
            "url": source_url.unwrap_or_else(|| api.url.clone()),
            "fetched_at": Utc::now().to_rfc3339(),
            "tables": tables,
            "key_values": key_values,
            "text_sample": text_sample,
            "text_sample_length": text_sample.len(),
            "raw_api": json_value,
        }))
    }

    fn convert_params(params: &Map<String, Value>) -> Vec<(String, String)> {
        params
            .iter()
            .map(|(k, v)| (k.clone(), Self::format_api_value(v)))
            .collect()
    }

    fn convert_api_response(
        value: &Value,
        cfg: &ApiQuoteConfig,
        max_rows: Option<usize>,
    ) -> (Vec<Value>, Vec<Value>) {
        const DEFAULT_RECORD_PATH: &[&str] = &["data", "diff"];
        let limit = max_rows.unwrap_or(50);
        let records_value = if let Some(path) = cfg.record_path.as_ref() {
            Self::traverse_path(value, path)
        } else {
            Self::traverse_default_path(value, DEFAULT_RECORD_PATH)
        };
        let records = records_value
            .and_then(|entry: &Value| entry.as_array())
            .cloned()
            .unwrap_or_default();
        if records.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let field_mappings = Self::resolve_field_mappings(cfg);
        let mut rows = Vec::new();
        for entry in records.iter().take(limit) {
            let mut row = Vec::new();
            for mapping in &field_mappings {
                let cell = Self::get_value_at_path(entry, &mapping.path)
                    .map(Self::format_api_value)
                    .unwrap_or_default();
                row.push(Value::String(cell));
            }
            rows.push(Value::Array(row));
        }

        let headers: Vec<Value> = field_mappings
            .iter()
            .map(|mapping| Value::String(mapping.column.clone()))
            .collect();
        let tables = if rows.is_empty() {
            Vec::new()
        } else {
            vec![json!({
                "headers": headers,
                "rows": rows,
                "source": "api",
            })]
        };

        let label_path = Self::resolve_label_path(cfg);
        let price_path = Self::resolve_price_path(cfg);
        let mut key_values = Vec::new();
        if let Some(first) = records.first() {
            if let Some(label_value) = Self::get_value_at_path(first, &label_path) {
                if let Some(price_value) = Self::get_value_at_path(first, &price_path) {
                    key_values.push(json!({
                        "label": Self::format_api_value(label_value),
                        "value": Self::format_api_value(price_value),
                    }));
                }
            }
        }

        (tables, key_values)
    }

    fn resolve_field_mappings(cfg: &ApiQuoteConfig) -> Vec<ApiFieldMapping> {
        if let Some(custom) = &cfg.field_mappings {
            return custom.clone();
        }
        vec![
            ApiFieldMapping {
                column: "名称".to_string(),
                path: vec![cfg.label_field.clone().unwrap_or_else(|| "f14".to_string())],
            },
            ApiFieldMapping {
                column: "最新价".to_string(),
                path: vec![cfg.price_field.clone().unwrap_or_else(|| "f2".to_string())],
            },
            ApiFieldMapping {
                column: "涨跌".to_string(),
                path: vec![cfg.change_field.clone().unwrap_or_else(|| "f4".to_string())],
            },
            ApiFieldMapping {
                column: "涨跌幅".to_string(),
                path: vec![cfg
                    .change_pct_field
                    .clone()
                    .unwrap_or_else(|| "f3".to_string())],
            },
        ]
    }

    fn resolve_label_path(cfg: &ApiQuoteConfig) -> Vec<String> {
        vec![cfg.label_field.clone().unwrap_or_else(|| "f14".to_string())]
    }

    fn resolve_price_path(cfg: &ApiQuoteConfig) -> Vec<String> {
        vec![cfg.price_field.clone().unwrap_or_else(|| "f2".to_string())]
    }

    fn traverse_path<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
        let mut current = value;
        for segment in path {
            current = current.get(segment)?;
        }
        Some(current)
    }

    fn traverse_default_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
        let mut current = value;
        for segment in path {
            current = current.get(segment)?;
        }
        Some(current)
    }

    fn get_value_at_path<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
        let mut current = value;
        for segment in path {
            current = current.get(segment)?;
        }
        Some(current)
    }

    fn format_api_value(value: &Value) -> String {
        match value {
            Value::String(s) => s.clone(),
            Value::Number(num) => num.to_string(),
            Value::Bool(flag) => flag.to_string(),
            other => other.to_string(),
        }
    }

    async fn execute_parse_tool(
        &self,
        context: &ToolExecutionContext,
        start: Instant,
        span: &tracing::Span,
        tool_id: &str,
    ) -> Result<ToolExecutionResult, SoulBrowserError> {
        let mut entry = OBSERVATION_CACHE
            .get_mut(&context.subject_id)
            .ok_or_else(|| {
                SoulBrowserError::internal(
                    "No observation available for this route; run data.extract-site first",
                )
            })?;

        let (schema, parsed) = self
            .run_data_parser(tool_id, &entry.data, &context.input)
            .map_err(|err| {
                self.record_error(context, start, span);
                err
            })?;
        entry.parsed.insert(schema.clone(), parsed.clone());
        drop(entry);

        let mut metadata = serde_json::Map::new();
        metadata.insert("schema".into(), json!(schema.clone()));
        let result = ToolExecutionResult {
            success: true,
            output: Some(json!({
                "status": "parsed",
                "schema": schema,
                "result": parsed,
            })),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
            metadata,
        };
        Ok(self.finish_tool(context, start, span, result))
    }

    async fn execute_metal_price_validation(
        &self,
        context: &ToolExecutionContext,
        start: Instant,
        span: &tracing::Span,
    ) -> Result<ToolExecutionResult, SoulBrowserError> {
        let payload = context
            .input
            .get("payload")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let payload: MetalPriceValidationPayload =
            serde_json::from_value(payload).map_err(|err| {
                self.record_error(context, start, span);
                SoulBrowserError::validation_error("invalid_payload", &err.to_string())
            })?;

        let entry = OBSERVATION_CACHE.get(&context.subject_id).ok_or_else(|| {
            SoulBrowserError::internal(
                "No observation available for validation; run data.parse.metal_price first",
            )
        })?;
        let parsed = entry.parsed.get("metal_price_v1").cloned().ok_or_else(|| {
            SoulBrowserError::validation_error(
                "missing_schema",
                "metal_price_v1 数据不存在，请先执行 data.parse.metal_price",
            )
        })?;
        drop(entry);

        let ctx = MetalPriceValidationContext {
            metal_keyword: payload
                .metal_keyword
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("铜"),
            allowed_markets: &payload.allowed_markets,
            max_age_hours: payload.max_age_hours,
        };

        let report = match validate_metal_price_with_context(&parsed, &ctx) {
            Ok(report) => report,
            Err(err) => {
                if let Some(failure) = err.downcast_ref::<MetalPriceValidationFailure>() {
                    match failure {
                        MetalPriceValidationFailure::MissingMarket(_)
                        | MetalPriceValidationFailure::MissingMetal(_) => {
                            emit_self_heal_event("switch_contract", Some(failure.to_string()));
                        }
                        MetalPriceValidationFailure::StaleQuotes { .. } => {
                            emit_self_heal_event("retry_alt_source", Some(failure.to_string()));
                        }
                    }
                }
                self.record_error(context, start, span);
                let message = err.to_string();
                return Err(SoulBrowserError::validation_error(
                    "metal_price_validation_failed",
                    &message,
                ));
            }
        };

        let mut metadata = serde_json::Map::new();
        if let Some(step_id) = payload.source_step_id {
            metadata.insert("source_step_id".into(), json!(step_id));
        }
        metadata.insert("schema".into(), json!("metal_price_v1"));
        metadata.insert("metal_keyword".into(), json!(ctx.metal_keyword));
        metadata.insert("fresh_entries".into(), json!(report.fresh_entries));

        let output = ToolExecutionResult {
            success: true,
            output: Some(json!({
                "status": "validated",
                "total_items": report.total_items,
                "matching_metal": report.matched_metal,
                "matching_market": report.matched_market,
                "fresh_entries": report.fresh_entries,
                "newest_as_of": report.newest_as_of,
            })),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
            metadata,
        };
        Ok(self.finish_tool(context, start, span, output))
    }

    async fn execute_target_validation(
        &self,
        context: &ToolExecutionContext,
        start: Instant,
        span: &tracing::Span,
    ) -> Result<ToolExecutionResult, SoulBrowserError> {
        let payload_value = context
            .input
            .get("payload")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let payload: TargetValidationPayload =
            serde_json::from_value(payload_value).map_err(|err| {
                self.record_error(context, start, span);
                SoulBrowserError::validation_error("invalid_payload", &err.to_string())
            })?;

        let TargetValidationPayload {
            source_step_id,
            keywords,
            allowed_domains,
            expected_status,
        } = payload;

        let entry = OBSERVATION_CACHE.get(&context.subject_id).ok_or_else(|| {
            SoulBrowserError::internal(
                "No observation available for validation; run data.extract-site first",
            )
        })?;
        let snapshot = entry.data.clone();
        drop(entry);

        let node = canonical_observation(&snapshot);
        let page_url = node
            .get("url")
            .and_then(Value::as_str)
            .map(|value| value.to_string());
        let title = node
            .get("title")
            .or_else(|| node.get("identity"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let text_sample = node
            .get("text_sample")
            .or_else(|| node.get("description"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if let Some(reason) = detect_block_reason(title, text_sample, page_url.as_deref()) {
            self.record_error(context, start, span);
            let detail = format!("页面疑似被拦截：{}", reason);
            return Err(target_validation_error(
                "target_validation_blocked",
                &detail,
            ));
        }

        let normalized_keywords = normalize_keywords(&keywords);
        let haystack = observation_text_blob(node);
        let matched_keywords: Vec<String> = normalized_keywords
            .iter()
            .filter(|keyword| keyword_matches(&haystack, keyword))
            .cloned()
            .collect();
        if !normalized_keywords.is_empty() && matched_keywords.is_empty() {
            self.record_error(context, start, span);
            let detail = format!("页面内容缺少目标关键词：{}", normalized_keywords.join(", "));
            return Err(target_validation_error(
                "target_validation_keywords_missing",
                &detail,
            ));
        }

        let normalized_domains = normalize_domains(&allowed_domains);
        let page_domain = page_url
            .as_deref()
            .and_then(|url| normalize_domain_input(url));
        if !normalized_domains.is_empty() {
            let domain_ok = page_domain.as_deref().map(|domain| {
                normalized_domains
                    .iter()
                    .any(|allowed| domain_matches_allowed(domain, allowed))
            });
            if !matches!(domain_ok, Some(true)) {
                self.record_error(context, start, span);
                let detail = format!(
                    "页面域名 {} 不在允许列表 {:?}",
                    page_domain.clone().unwrap_or_else(|| "unknown".to_string()),
                    normalized_domains
                );
                return Err(target_validation_error(
                    "target_validation_domain_mismatch",
                    &detail,
                ));
            }
        }

        let status_code = extract_status_code(node);
        if let (Some(expected), Some(actual)) = (expected_status, status_code) {
            if expected != actual {
                self.record_error(context, start, span);
                let detail = format!("HTTP 状态 {} 与期望 {} 不符", actual, expected);
                return Err(target_validation_error(
                    "target_validation_status_mismatch",
                    &detail,
                ));
            }
        }

        let mut metadata = serde_json::Map::new();
        if let Some(step_id) = source_step_id.as_deref() {
            metadata.insert("source_step_id".into(), json!(step_id));
        }
        if !normalized_keywords.is_empty() {
            metadata.insert("keywords".into(), json!(normalized_keywords));
        }
        metadata.insert("matched_keywords".into(), json!(matched_keywords.clone()));
        if !normalized_domains.is_empty() {
            metadata.insert("allowed_domains".into(), json!(normalized_domains));
        }
        if let Some(domain) = page_domain.as_deref() {
            metadata.insert("domain".into(), json!(domain));
        }
        if let Some(status) = status_code {
            metadata.insert("status_code".into(), json!(status));
        }
        if let Some(expected) = expected_status {
            metadata.insert("expected_status".into(), json!(expected));
        }

        let result = ToolExecutionResult {
            success: true,
            output: Some(json!({
                "status": "validated",
                "matched_keywords": matched_keywords,
                "domain": page_domain,
                "status_code": status_code,
            })),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
            metadata,
        };
        Ok(self.finish_tool(context, start, span, result))
    }

    async fn execute_deliver_tool(
        &self,
        context: &ToolExecutionContext,
        start: Instant,
        span: &tracing::Span,
    ) -> Result<ToolExecutionResult, SoulBrowserError> {
        let schema = required_deliver_field(
            &context.input,
            "schema",
            "deliver_missing_schema",
            "payload.schema 缺失，请补 generic_observation_v1 或对应解析 schema",
        )?;
        let entry = OBSERVATION_CACHE
            .get_mut(&context.subject_id)
            .ok_or_else(|| {
                SoulBrowserError::internal(
                    "No parsed observation available; parse before delivering",
                )
            })?;
        let parsed_value = entry
            .parsed
            .get(&schema)
            .or_else(|| entry.parsed.values().next())
            .cloned()
            .ok_or_else(|| {
                SoulBrowserError::internal(
                    "Missing parsed data for requested schema; ensure data.parse.* ran",
                )
            })?;
        drop(entry);

        validate_structured_output(&schema, &parsed_value).map_err(|err| {
            self.record_error(context, start, span);
            SoulBrowserError::validation_error("Invalid structured output", &err.to_string())
        })?;

        let task_id = context
            .input
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("anonymous");
        let artifact_label = required_deliver_field(
            &context.input,
            "artifact_label",
            "deliver_missing_artifact_label",
            "payload.artifact_label 缺失，用于命名结构化产物，请填写例如 structured.github_repos_v1",
        )?;
        let filename_value = required_deliver_field(
            &context.input,
            "filename",
            "deliver_missing_filename",
            "payload.filename 缺失，请指定例如 github_repos_v1.json",
        )?;
        let filename = PathBuf::from(filename_value.as_str());
        let source_step_id = required_deliver_field(
            &context.input,
            "source_step_id",
            "deliver_missing_source_step",
            "payload.source_step_id 缺失，请引用前序解析步骤 ID",
        )?;

        let root = PathBuf::from("soulbrowser-output")
            .join("artifacts")
            .join(task_id);
        fs::create_dir_all(&root).await.map_err(|err| {
            SoulBrowserError::internal(&format!("Failed to prepare artifact dir: {}", err))
        })?;
        let artifact_path = root.join(&filename);
        let payload = serde_json::to_vec_pretty(&parsed_value).map_err(|err| {
            SoulBrowserError::internal(&format!("Failed to serialize structured output: {}", err))
        })?;
        fs::write(&artifact_path, payload).await.map_err(|err| {
            SoulBrowserError::internal(&format!("Failed to write structured artifact: {}", err))
        })?;
        OBSERVATION_CACHE.remove(&context.subject_id);

        let artifact_path_str = artifact_path.to_string_lossy().to_string();
        let mut metadata = serde_json::Map::new();
        metadata.insert("schema".into(), json!(schema.clone()));
        metadata.insert("artifact_label".into(), json!(artifact_label.clone()));
        metadata.insert("source_step_id".into(), json!(source_step_id.clone()));
        metadata.insert("artifact_path".into(), json!(artifact_path_str.clone()));

        let mut output_payload = Map::new();
        output_payload.insert("status".into(), json!("delivered"));
        output_payload.insert("schema".into(), json!(schema.clone()));
        output_payload.insert("artifact_label".into(), json!(artifact_label.clone()));
        output_payload.insert("source_step_id".into(), json!(source_step_id.clone()));
        output_payload.insert("artifact_path".into(), json!(artifact_path_str.clone()));
        if let Some(summary) = summarize_structured_output(&schema, &parsed_value) {
            output_payload.insert("summary".into(), json!(summary.clone()));
            metadata.insert("summary".into(), json!(summary));
        }

        let result = ToolExecutionResult {
            success: true,
            output: Some(Value::Object(output_payload)),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
            metadata,
        };
        Ok(self.finish_tool(context, start, span, result))
    }

    fn run_data_parser(
        &self,
        tool_id: &str,
        observation: &Value,
        payload: &Value,
    ) -> Result<(String, Value), SoulBrowserError> {
        match tool_id {
            "data.parse.generic" => {
                let schema = payload
                    .get("schema")
                    .and_then(|v| v.as_str())
                    .unwrap_or("generic_observation_v1");
                Ok((
                    schema.to_string(),
                    parse_generic_observation(observation, schema),
                ))
            }
            "data.parse.market_info" => parse_market_info(observation)
                .map(|value| ("market_info_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse market info failed: {}", err))
                }),
            "data.parse.metal_price" => parse_metal_price(observation)
                .map(|value| ("metal_price_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse metal price failed: {}", err))
                }),
            "data.parse.news_brief" => parse_news_brief(observation)
                .map(|value| ("news_brief_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse news brief failed: {}", err))
                }),
            "data.parse.twitter-feed" => parse_twitter_feed(observation)
                .map(|value| ("twitter_feed_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse twitter feed failed: {}", err))
                }),
            "data.parse.weather" => parse_weather(observation)
                .map(|value| ("weather_report_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse weather failed: {}", err))
                }),
            "data.parse.facebook-feed" => parse_facebook_feed(observation)
                .map(|value| ("facebook_feed_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse facebook feed failed: {}", err))
                }),
            "data.parse.hackernews-feed" => parse_hackernews_feed(observation)
                .map(|value| ("hackernews_feed_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse hackernews feed failed: {}", err))
                }),
            "data.parse.linkedin-profile" => parse_linkedin_profile(observation)
                .map(|value| ("linkedin_profile_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse linkedin profile failed: {}", err))
                }),
            "data.parse.github-repo" => {
                let username = payload
                    .get("username")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if username.trim().is_empty() {
                    return Err(SoulBrowserError::validation_error(
                        "Missing field",
                        "'username' is required for data.parse.github-repo",
                    ));
                }
                parse_github_repos(observation, username)
                    .map(|value| ("github_repos_v1".to_string(), value))
                    .map_err(|err| {
                        SoulBrowserError::internal(&format!(
                            "Parse github repositories failed: {}",
                            err
                        ))
                    })
            }
            other => Err(SoulBrowserError::internal(&format!(
                "Unknown parse tool '{}'; supported tools must be registered",
                other
            ))),
        }
    }

    fn parse_anchor(value: &serde_json::Value) -> Result<AnchorDescriptor, SoulBrowserError> {
        if let Some(s) = value.as_str() {
            if s.trim().is_empty() {
                return Err(SoulBrowserError::validation_error(
                    "Invalid anchor",
                    "CSS selector cannot be empty",
                ));
            }
            return Ok(AnchorDescriptor::Css(s.to_string()));
        }

        let obj = value.as_object().ok_or_else(|| {
            SoulBrowserError::validation_error(
                "Invalid anchor",
                "Anchor must be a string or object",
            )
        })?;

        if let Some(selector) = obj.get("selector").and_then(|v| v.as_str()) {
            let strategy = obj
                .get("strategy")
                .or_else(|| obj.get("kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("css");
            return match strategy.to_ascii_lowercase().as_str() {
                "css" => Ok(AnchorDescriptor::Css(selector.to_string())),
                "text" => {
                    let exact = obj.get("exact").and_then(|v| v.as_bool()).unwrap_or(false);
                    Ok(AnchorDescriptor::Text {
                        content: selector.to_string(),
                        exact,
                    })
                }
                other => Err(SoulBrowserError::validation_error(
                    "Invalid anchor strategy",
                    &format!("Unsupported anchor strategy '{}'.", other),
                )),
            };
        }

        if let (Some(role), Some(name)) = (
            obj.get("role").and_then(|v| v.as_str()),
            obj.get("name").and_then(|v| v.as_str()),
        ) {
            if role.is_empty() || name.is_empty() {
                return Err(SoulBrowserError::validation_error(
                    "Invalid anchor",
                    "role/name cannot be empty",
                ));
            }
            return Ok(AnchorDescriptor::Aria {
                role: role.to_string(),
                name: name.to_string(),
            });
        }

        if let Some(content) = obj.get("text").and_then(|v| v.as_str()) {
            let exact = obj.get("exact").and_then(|v| v.as_bool()).unwrap_or(false);
            if content.is_empty() {
                return Err(SoulBrowserError::validation_error(
                    "Invalid anchor",
                    "Text anchor cannot be empty",
                ));
            }
            return Ok(AnchorDescriptor::Text {
                content: content.to_string(),
                exact,
            });
        }

        Err(SoulBrowserError::validation_error(
            "Invalid anchor",
            "Unrecognized anchor descriptor",
        ))
    }

    fn parse_scroll_behavior(
        value: Option<&serde_json::Value>,
    ) -> Result<ScrollBehavior, SoulBrowserError> {
        if let Some(val) = value {
            if let Some(kind) = val.as_str() {
                return match kind.to_ascii_lowercase().as_str() {
                    "smooth" => Ok(ScrollBehavior::Smooth),
                    "instant" => Ok(ScrollBehavior::Instant),
                    "natural" => Ok(ScrollBehavior::Smooth),
                    other => Err(SoulBrowserError::validation_error(
                        "Invalid scroll behavior",
                        &format!("Unsupported scroll behavior '{}'.", other),
                    )),
                };
            }
        }
        Ok(ScrollBehavior::default())
    }

    fn parse_scroll_target(value: &serde_json::Value) -> Result<ScrollTarget, SoulBrowserError> {
        if let Some(kind_value) = value.get("kind").and_then(|v| v.as_str()) {
            match kind_value.to_ascii_lowercase().as_str() {
                "top" => return Ok(ScrollTarget::Top),
                "bottom" => return Ok(ScrollTarget::Bottom),
                "pixels" | "delta" => {
                    let amount = value
                        .get("value")
                        .or_else(|| value.get("delta"))
                        .and_then(|v| v.as_i64())
                        .ok_or_else(|| {
                            SoulBrowserError::validation_error(
                                "Invalid scroll target",
                                "Delta scroll requires numeric value",
                            )
                        })?;
                    return Ok(ScrollTarget::Pixels(amount as i32));
                }
                "element" | "elementcenter" => {
                    let anchor_val = value.get("anchor").ok_or_else(|| {
                        SoulBrowserError::validation_error(
                            "Invalid scroll target",
                            "Element scroll requires anchor descriptor",
                        )
                    })?;
                    let anchor = Self::parse_anchor(anchor_val)?;
                    return Ok(ScrollTarget::Element(anchor));
                }
                "toy" => {
                    let y = value.get("value").and_then(|v| v.as_i64()).ok_or_else(|| {
                        SoulBrowserError::validation_error(
                            "Invalid scroll target",
                            "toY target requires numeric value",
                        )
                    })?;
                    return Ok(ScrollTarget::Pixels(y as i32));
                }
                other => {
                    return Err(SoulBrowserError::validation_error(
                        "Invalid scroll target",
                        &format!("Unsupported target kind '{}'.", other),
                    ))
                }
            }
        }

        if let Some(anchor_val) = value.get("anchor") {
            let anchor = Self::parse_anchor(anchor_val)?;
            return Ok(ScrollTarget::Element(anchor));
        }

        Err(SoulBrowserError::validation_error(
            "Invalid scroll target",
            "Target must specify kind or anchor",
        ))
    }

    fn parse_wait_condition_for_element(
        target: &serde_json::Value,
        condition: &serde_json::Value,
    ) -> Result<WaitCondition, SoulBrowserError> {
        let anchor_val = target.get("anchor").unwrap_or(target);
        let anchor = Self::parse_anchor(anchor_val)?;

        let kind = condition
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("visible");

        match kind.to_ascii_lowercase().as_str() {
            "visible" | "clickable" | "present" => Ok(WaitCondition::ElementVisible(anchor)),
            "hidden" | "removed" => Ok(WaitCondition::ElementHidden(anchor)),
            other => Err(SoulBrowserError::validation_error(
                "Unsupported condition",
                &format!(
                    "Condition '{}' is not supported yet. Supported: visible, clickable, present, hidden, removed.",
                    other
                ),
            )),
        }
    }

    fn parse_wait_condition_for_expect(
        expect: &serde_json::Value,
    ) -> Result<WaitCondition, SoulBrowserError> {
        if let Some(pattern) = expect.get("url_pattern").and_then(|v| v.as_str()) {
            return Ok(WaitCondition::UrlMatches(pattern.to_string()));
        }

        if let Some(pattern) = expect.get("title_pattern").and_then(|v| v.as_str()) {
            return Ok(WaitCondition::TitleMatches(pattern.to_string()));
        }

        if let Some(expected) = expect.get("url_equals").and_then(|v| v.as_str()) {
            return Ok(WaitCondition::UrlEquals(expected.to_string()));
        }

        if let Some(net) = expect.get("net") {
            if let Some(quiet_ms) = net.get("quiet_ms").and_then(|v| v.as_u64()) {
                return Ok(WaitCondition::NetworkIdle(quiet_ms));
            }
        }

        if let Some(duration_ms) = expect
            .get("duration_ms")
            .or_else(|| expect.get("sleep_ms"))
            .and_then(|v| v.as_u64())
        {
            return Ok(WaitCondition::Duration(duration_ms));
        }

        Err(SoulBrowserError::validation_error(
            "Unsupported expect",
            "wait-for-condition currently supports net.quiet_ms, duration_ms, url_pattern, url_equals, or title_pattern",
        ))
    }

    async fn pointer_click(
        &self,
        route: &ExecRoute,
        x: f64,
        y: f64,
        button: &str,
    ) -> Result<(), SoulBrowserError> {
        let page_id = self.resolve_page_for_route(route).await?;
        let normalized_button = match button.to_ascii_lowercase().as_str() {
            "right" => "right",
            "middle" => "middle",
            _ => "left",
        };
        let payload = |event_type: &str| {
            json!({
                "type": event_type,
                "x": x,
                "y": y,
                "button": normalized_button,
                "buttons": 1,
                "clickCount": 1,
                "pointerType": "mouse",
            })
        };
        self.adapter
            .dispatch_mouse_event(page_id, payload("mousePressed"))
            .await
            .map_err(|err| Self::pointer_error("click", err))?;
        self.adapter
            .dispatch_mouse_event(page_id, payload("mouseReleased"))
            .await
            .map_err(|err| Self::pointer_error("click", err))
    }

    async fn pointer_scroll(
        &self,
        route: &ExecRoute,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<(), SoulBrowserError> {
        let page_id = self.resolve_page_for_route(route).await?;
        let payload = json!({
            "type": "mouseWheel",
            "x": x,
            "y": y,
            "deltaX": delta_x,
            "deltaY": delta_y,
            "pointerType": "mouse",
        });
        self.adapter
            .dispatch_mouse_event(page_id, payload)
            .await
            .map_err(|err| Self::pointer_error("scroll", err))
    }

    async fn pointer_type(&self, route: &ExecRoute, text: &str) -> Result<(), SoulBrowserError> {
        let page_id = self.resolve_page_for_route(route).await?;
        self.adapter
            .insert_text_event(page_id, text)
            .await
            .map_err(|err| Self::pointer_error("type", err))
    }

    fn pointer_error(action: &str, err: AdapterError) -> SoulBrowserError {
        SoulBrowserError::internal(&format!("manual pointer {action} failed: {}", err))
    }
}

fn required_deliver_field(
    input: &Value,
    key: &str,
    code: &'static str,
    remedial: &'static str,
) -> Result<String, SoulBrowserError> {
    input
        .get(key)
        .and_then(|value| value.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| SoulBrowserError::validation_error(code, remedial))
}

fn required_field(
    input: &Value,
    key: &str,
    code: &'static str,
    remedial: &'static str,
) -> Result<String, SoulBrowserError> {
    input
        .get(key)
        .and_then(|value| value.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| SoulBrowserError::validation_error(code, remedial))
}

fn build_weather_search_url(query: &str, override_url: Option<&str>) -> String {
    if let Some(custom) = override_url {
        if !custom.trim().is_empty() {
            return custom.to_string();
        }
    }
    let encoded: String = form_urlencoded::byte_serialize(query.trim().as_bytes())
        .collect::<String>()
        .replace('+', "%20");
    format!("https://www.baidu.com/s?wd={}", encoded)
}

fn normalized_search_engine(engine: &str) -> &'static str {
    match engine.to_ascii_lowercase().as_str() {
        "bing" => "bing",
        "google" => "google",
        "duckduckgo" | "ddg" => "duckduckgo",
        "baidu" => "baidu",
        // Default to DuckDuckGo as it has fewer captchas
        _ => "duckduckgo",
    }
}

fn search_engine_attempts(engine_hint: &str) -> Vec<&'static str> {
    let preferred = normalized_search_engine(engine_hint);
    let mut order = vec![preferred];
    // DuckDuckGo first as fallback (fewer captchas), then others
    for fallback in ["duckduckgo", "google", "bing", "baidu"].iter() {
        if order.contains(fallback) {
            continue;
        }
        order.push(fallback);
    }
    order
}

fn build_browser_search_url(
    engine_hint: &str,
    query: &str,
    site: Option<&str>,
    override_url: Option<&str>,
) -> (String, String) {
    if let Some(custom) = override_url {
        if !custom.trim().is_empty() {
            return (
                normalized_search_engine(engine_hint).to_string(),
                custom.to_string(),
            );
        }
    }
    let normalized = normalized_search_engine(engine_hint);
    let mut final_query = query.trim().to_string();
    if let Some(site_value) = site.and_then(|s| normalize_site_hint(s)) {
        if !final_query.is_empty() {
            final_query.push(' ');
        }
        final_query.push_str(&format!("site:{}", site_value));
    }
    let encoded: String = form_urlencoded::byte_serialize(final_query.as_bytes()).collect();
    let url = match normalized {
        "bing" => format!("https://www.bing.com/search?q={encoded}"),
        "google" => format!("https://www.google.com/search?q={encoded}"),
        "duckduckgo" => format!("https://duckduckgo.com/?q={encoded}"),
        "baidu" => format!("https://www.baidu.com/s?wd={encoded}"),
        _ => format!("https://duckduckgo.com/?q={encoded}"),
    };
    (normalized.to_string(), url)
}

fn default_search_results_selector(engine: &str) -> &'static str {
    search_selectors(engine)
        .first()
        .copied()
        .unwrap_or("div#content_left")
}

fn search_selectors(engine: &str) -> &'static [&'static str] {
    match engine {
        "bing" => BING_SEARCH_SELECTORS,
        "google" => GOOGLE_SEARCH_SELECTORS,
        "duckduckgo" => DUCKDUCKGO_SEARCH_SELECTORS,
        "baidu" => BAIDU_SEARCH_SELECTORS,
        _ => DUCKDUCKGO_SEARCH_SELECTORS,
    }
}

fn collect_results_selectors(input: &Value, engine: &str) -> Vec<String> {
    let mut selectors: Vec<String> = Vec::new();
    if let Some(value) = input.get("results_selectors") {
        append_selector_value(value, &mut selectors);
    }
    if let Some(selector) = input
        .get("results_selector")
        .and_then(Value::as_str)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        push_selector(selector, &mut selectors);
    }
    for selector in search_selectors(engine) {
        push_selector(selector, &mut selectors);
    }
    selectors
}

fn append_selector_value(value: &Value, selectors: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                if let Some(text) = item.as_str() {
                    push_selector(text, selectors);
                }
            }
        }
        Value::String(text) => push_selector(text, selectors),
        _ => {}
    }
}

fn push_selector(text: &str, selectors: &mut Vec<String>) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    if selectors.iter().any(|existing| existing == trimmed) {
        return;
    }
    selectors.push(trimmed.to_string());
}

fn auto_act_result_picker_script(config: &Value) -> Result<String, SoulBrowserError> {
    serde_json::to_string(config)
        .map(|serialized| AUTO_ACT_RESULT_PICKER_SCRIPT.replace("__CONFIG__", &serialized))
        .map_err(|err| {
            SoulBrowserError::internal(&format!(
                "Failed to serialize AutoAct result picker config: {}",
                err
            ))
        })
}

fn auto_act_candidate_selectors(input: &Value, engine: &str) -> Vec<String> {
    let mut selectors = Vec::new();
    if let Some(value) = input.get("selectors") {
        append_selector_value(value, &mut selectors);
    }
    if selectors.is_empty() {
        if let Some(value) = input.get("results_selectors") {
            append_selector_value(value, &mut selectors);
        }
    }
    if selectors.is_empty() {
        if let Some(value) = input.get("results_selector").and_then(Value::as_str) {
            push_selector(value, &mut selectors);
        }
    }
    if selectors.is_empty() {
        for selector in search_selectors(engine) {
            selectors.push(selector.to_string());
        }
    }
    selectors
        .into_iter()
        .map(|selector| selector.trim().to_string())
        .filter(|selector| !selector.is_empty())
        .collect()
}

fn auto_act_domain_hints(input: &Value) -> Vec<String> {
    let mut domains = Vec::new();
    if let Some(value) = input.get("domains") {
        append_selector_value(value, &mut domains);
    }
    domains
        .into_iter()
        .filter_map(|raw| canonical_domain_hint(&raw))
        .collect()
}

fn canonical_domain_hint(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(parsed) = Url::parse(trimmed) {
        if let Some(domain) = parsed.domain() {
            return Some(domain.to_ascii_lowercase());
        }
    }
    let without_scheme = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("//");
    let without_glob = without_scheme
        .trim_start_matches('*')
        .trim_start_matches('.');
    let cleaned = without_glob.trim_matches('/');
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_ascii_lowercase())
    }
}

fn domain_pattern(domain: &str) -> Option<String> {
    let trimmed = domain.trim().trim_start_matches('*').trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    Some(format!("https?://[^/]*{}.*", escape(trimmed)))
}

fn url_prefix_pattern(url: &str) -> Option<String> {
    if url.trim().is_empty() {
        return None;
    }
    if let Ok(parsed) = Url::parse(url) {
        let mut base = format!("{}://{}", parsed.scheme(), parsed.host_str()?);
        if let Some(port) = parsed.port() {
            base.push(':');
            base.push_str(&port.to_string());
        }
        base.push_str(parsed.path());
        let escaped = escape(&base);
        return Some(format!("^{}.*", escaped));
    }
    Some(format!("^{}.*", escape(url)))
}

fn post_click_patterns(
    selected_url: Option<&str>,
    matched_domain: Option<&str>,
    domain_hints: &[String],
    engine: &str,
) -> Vec<String> {
    if let Some(url) = selected_url {
        let prefer_guardrail =
            should_prioritize_guardrail(Some(url), matched_domain, domain_hints, engine);
        if prefer_guardrail {
            let mut patterns: Vec<String> = domain_hints
                .iter()
                .filter_map(|domain| domain_pattern(domain))
                .collect();
            if let Some(pattern) = url_prefix_pattern(url) {
                patterns.push(pattern);
            }
            for fallback in fallback_wait_patterns(engine) {
                if !patterns.iter().any(|existing| existing == &fallback) {
                    patterns.push(fallback);
                }
            }
            if !patterns.is_empty() {
                patterns.dedup();
                return patterns;
            }
        } else if let Some(pattern) = url_prefix_pattern(url) {
            return vec![pattern];
        }
    }
    if let Some(domain) = matched_domain {
        if let Some(pattern) = domain_pattern(domain) {
            return vec![pattern];
        }
    }
    let mut patterns = Vec::new();
    for domain in domain_hints {
        if let Some(pattern) = domain_pattern(domain) {
            patterns.push(pattern);
        }
    }
    if !patterns.is_empty() {
        patterns.dedup();
        return patterns;
    }
    fallback_wait_patterns(engine)
}

fn is_baidu_redirect(url: &str) -> bool {
    if let Ok(parsed) = Url::parse(url) {
        if let Some(host) = parsed.host_str().map(|host| host.to_ascii_lowercase()) {
            if host.ends_with("baidu.com") && parsed.path().starts_with("/link") {
                return true;
            }
        }
    }
    false
}

fn fallback_wait_patterns(engine: &str) -> Vec<String> {
    if engine.eq_ignore_ascii_case("baidu") {
        vec![BAIDU_RESULT_REDIRECT_PATTERN.to_string()]
    } else {
        Vec::new()
    }
}

fn should_prioritize_guardrail(
    selected_url: Option<&str>,
    matched_domain: Option<&str>,
    domain_hints: &[String],
    engine: &str,
) -> bool {
    let Some(url) = selected_url else {
        return false;
    };
    matched_domain.is_none()
        && !domain_hints.is_empty()
        && engine.eq_ignore_ascii_case("baidu")
        && is_baidu_redirect(url)
}

fn pattern_targets_guardrail(pattern: &str, domain_hints: &[String]) -> bool {
    domain_hints
        .iter()
        .any(|domain| pattern.contains(&escape(domain)))
}

fn dynamic_wait_timeout(
    pattern: &str,
    domain_hints: &[String],
    engine: &str,
    default_ms: u64,
) -> u64 {
    if engine.eq_ignore_ascii_case("baidu") && pattern_targets_guardrail(pattern, domain_hints) {
        default_ms.min(5_000).max(1_000)
    } else {
        default_ms
    }
}

fn search_url_wait_condition(engine: &str, url: &str) -> WaitCondition {
    if let Ok(parsed) = Url::parse(url) {
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
        let mut pattern = format!("^{}", escape(&base));
        let param = match engine {
            "bing" | "google" => "q",
            _ => "wd",
        };
        if let Some((_, value)) = parsed.query_pairs().find(|(key, _)| key == param) {
            let encoded: String = form_urlencoded::byte_serialize(value.as_bytes()).collect();
            pattern.push_str(".*");
            pattern.push_str(&escape(param));
            pattern.push('=');
            pattern.push_str(&escape(&encoded));
        }
        pattern.push_str(".*$");
        return WaitCondition::UrlMatches(pattern);
    }
    WaitCondition::UrlMatches(format!(".*{}.*", escape(engine)))
}

fn normalize_site_hint(site: &str) -> Option<String> {
    let trimmed = site.trim();
    if trimmed.is_empty() {
        return None;
    }
    let cleaned = trimmed.trim_start_matches("site:").trim();
    if cleaned.is_empty() {
        return None;
    }
    if let Ok(parsed) = Url::parse(cleaned) {
        if let Some(host) = parsed.host_str().map(|h| h.to_string()) {
            return Some(host);
        }
    }
    let without_scheme = cleaned
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host_part = without_scheme
        .split(|c| matches!(c, '/' | '?'))
        .next()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())?;
    Some(host_part.to_string())
}

fn modal_selectors_from_input(input: &Value) -> Vec<String> {
    if let Some(selector) = input
        .get("selector")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return vec![selector.to_string()];
    }
    if let Some(list) = input.get("selectors").and_then(|v| v.as_array()) {
        let collected: Vec<String> = list
            .iter()
            .filter_map(|value| value.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        if !collected.is_empty() {
            return collected;
        }
    }
    DEFAULT_MODAL_CLOSE_SELECTORS
        .iter()
        .map(|selector| selector.to_string())
        .collect()
}

fn keyboard_event_script(
    key: &str,
    code: &str,
    key_code: u32,
    focus_selector: Option<&str>,
) -> String {
    let key_literal = serde_json::to_string(key).unwrap_or_else(|_| "\"Escape\"".to_string());
    let code_literal = serde_json::to_string(code).unwrap_or_else(|_| "\"Escape\"".to_string());
    let focus_literal = focus_selector
        .map(|selector| serde_json::to_string(selector).unwrap_or_else(|_| "null".to_string()))
        .unwrap_or_else(|| "null".to_string());
    format!(
        "(() => {{\n            const focusSelector = {focus};\n            const options = {{ key: {key}, code: {code}, keyCode: {key_code}, which: {key_code}, bubbles: true }};\n            const targets = [];\n            if (focusSelector) {{\n                const candidate = document.querySelector(focusSelector);\n                if (candidate) {{\n                    try {{ candidate.focus(); }} catch (err) {{}}\n                    targets.push(candidate);\n                }}\n            }}\n            const active = document.activeElement;\n            if (active && !targets.includes(active)) {{\n                targets.push(active);\n            }}\n            if (document.body && !targets.includes(document.body)) {{\n                targets.push(document.body);\n            }}\n            if (document.documentElement && !targets.includes(document.documentElement)) {{\n                targets.push(document.documentElement);\n            }}\n            targets.push(window);\n            let dispatched = false;\n            for (const target of targets) {{\n                if (!target) continue;\n                const down = new KeyboardEvent('keydown', options);\n                const up = new KeyboardEvent('keyup', options);\n                target.dispatchEvent(down);\n                target.dispatchEvent(up);\n                dispatched = true;\n            }}\n            return dispatched;\n        }})()",
        key = key_literal,
        code = code_literal,
        key_code = key_code,
        focus = focus_literal,
    )
}

fn weather_wait_condition(url: &str) -> WaitCondition {
    if let Ok(parsed) = Url::parse(url) {
        if let Some((_, value)) = parsed.query_pairs().find(|(key, _)| key == "wd") {
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
            let encoded: String = form_urlencoded::byte_serialize(value.as_bytes()).collect();
            let mut pattern = format!("^{}.*", escape(&base));
            pattern.push_str(&format!("wd={}.*", escape(&encoded)));
            pattern.push('$');
            return WaitCondition::UrlMatches(pattern);
        }
    }
    WaitCondition::UrlEquals(url.to_string())
}

fn build_weather_link_patterns(input: &Value) -> Vec<String> {
    let mut patterns = Vec::new();
    if let Some(value) = input
        .get("preferred_link_substrings")
        .and_then(|v| v.as_array())
    {
        for entry in value {
            if let Some(text) = entry.as_str() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    let lowered = trimmed.to_ascii_lowercase();
                    if !patterns.contains(&lowered) {
                        patterns.push(lowered);
                    }
                }
            }
        }
    }
    if let Some(text) = input
        .get("preferred_link_substring")
        .and_then(|v| v.as_str())
    {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            let lowered = trimmed.to_ascii_lowercase();
            if !patterns.contains(&lowered) {
                patterns.push(lowered);
            }
        }
    }
    const FALLBACKS: &[&str] = &[
        "moji.com",
        "墨迹",
        "tianqi.com",
        "中国天气",
        "weather.com",
        "weather.com.cn",
        "weathercn.com",
        "天气网",
    ];
    for fallback in FALLBACKS {
        let lowered = fallback.to_ascii_lowercase();
        if !patterns.contains(&lowered) {
            patterns.push(lowered);
        }
    }
    patterns
}

fn is_trusted_weather_url(url: &str, preferred_override: Option<&str>) -> bool {
    if let Ok(parsed) = Url::parse(url) {
        if let Some(host) = parsed.host_str().map(|h| h.to_ascii_lowercase()) {
            if TRUSTED_WEATHER_DOMAINS
                .iter()
                .any(|domain| host.ends_with(domain))
            {
                return true;
            }
        }
    }
    if let Some(extra) = preferred_override {
        let trimmed = extra.trim();
        if !trimmed.is_empty() {
            let needle = trimmed.to_ascii_lowercase();
            return url.to_ascii_lowercase().contains(&needle);
        }
    }
    false
}

struct PageHealthSnapshot {
    url: String,
    title: String,
    text_sample: String,
}

impl PageHealthSnapshot {
    fn as_value(&self) -> Value {
        json!({
            "url": self.url,
            "title": self.title,
            "text_sample": self.text_sample,
        })
    }
}

#[derive(Debug, Clone)]
struct WeatherCandidateOutcome {
    final_url: String,
    snapshot: Value,
}

enum WeatherCandidateEvaluation {
    Ready(WeatherCandidateOutcome),
    Blocked(String),
}

impl BrowserToolExecutor {
    async fn extract_weather_result_links(
        &self,
        exec_ctx: &ExecCtx,
        patterns: &[String],
        limit: usize,
        preferred_override: Option<&str>,
    ) -> Result<Vec<String>, SoulBrowserError> {
        if patterns.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let context = self
            .primitives
            .resolve_context(exec_ctx)
            .await
            .map_err(|err| {
                SoulBrowserError::internal(&format!("Failed to resolve execution context: {}", err))
            })?;
        let pattern_literal = serde_json::to_string(patterns).map_err(|err| {
            SoulBrowserError::internal(&format!("Invalid substring list: {}", err))
        })?;
        let template = r#"(() => {
                const patterns = __PATTERNS__.map(p => (p || '').toLowerCase().trim()).filter(Boolean);
                const anchors = Array.from(document.querySelectorAll('#content_left a[href]'));
                const decodeCandidate = raw => {
                    if (!raw) { return ''; }
                    try {
                        const parsed = new URL(raw, document.location.href);
                        const param = parsed.searchParams.get('url');
                        if (param) {
                            let decoded = param;
                            try { decoded = decodeURIComponent(param); } catch (err) { decoded = param; }
                            if (/^https?:\/\//i.test(decoded)) {
                                return decoded;
                            }
                        }
                        return parsed.href;
                    } catch (err) {
                        return raw;
                    }
                };
                const matches = [];
                const seen = new Set();
                for (const anchor of anchors) {
                    if (matches.length >= __LIMIT__) { break; }
                    const hrefAttr = (anchor.getAttribute('href') || '').trim();
                    const dataAttr = (anchor.getAttribute('data-landurl') || anchor.getAttribute('data-url') || '').trim();
                    const text = (anchor.innerText || '').toLowerCase();
                    const haystacks = [hrefAttr.toLowerCase(), dataAttr.toLowerCase(), text];
                    const matched = patterns.length === 0 || patterns.some(pattern => haystacks.some(h => h.includes(pattern)));
                    if (!matched) { continue; }
                    const raw = dataAttr || hrefAttr;
                    if (!raw) { continue; }
                    const candidate = decodeCandidate(raw);
                    if (!candidate) { continue; }
                    let absolute;
                    try {
                        absolute = new URL(candidate, document.location.href).href;
                    } catch (err) {
                        continue;
                    }
                    if (!/^https?:\/\//i.test(absolute)) { continue; }
                    if (seen.has(absolute)) { continue; }
                    seen.add(absolute);
                    matches.push(absolute);
                }
                return matches;
            })()"#;
        let script = template
            .replace("__PATTERNS__", &pattern_literal)
            .replace("__LIMIT__", &limit.to_string());
        let value = self
            .primitives
            .adapter()
            .evaluate_script_in_context(&context, &script)
            .await
            .map_err(|err| {
                SoulBrowserError::internal(&format!(
                    "Failed to inspect weather result link: {}",
                    err
                ))
            })?;
        if let Some(array) = value.as_array() {
            if array.is_empty() {
                warn!(target: "weather.search", "Baidu result scan yielded 0 anchors");
            }
            let mut collected = Vec::new();
            for entry in array {
                if let Some(url) = entry.as_str() {
                    if is_trusted_weather_url(url, preferred_override) {
                        collected.push(url.to_string());
                    }
                }
            }
            if collected.is_empty() {
                warn!(target: "weather.search", "No trusted weather domains found; falling back to generic anchors");
                for entry in array {
                    if let Some(url) = entry.as_str() {
                        collected.push(url.to_string());
                    }
                    if collected.len() >= limit {
                        break;
                    }
                }
            }
            return Ok(collected);
        }
        Ok(Vec::new())
    }

    async fn capture_page_snapshot(
        &self,
        exec_ctx: &ExecCtx,
    ) -> Result<PageHealthSnapshot, SoulBrowserError> {
        let context = self
            .primitives
            .resolve_context(exec_ctx)
            .await
            .map_err(|err| {
                SoulBrowserError::internal(&format!("Failed to resolve execution context: {}", err))
            })?;
        let script = "(() => { const title = document.title || ''; const text = (document.body && document.body.innerText) || ''; return { url: window.location.href || '', title, text: text.replace(/\\s+/g, ' ').slice(0, 4000) }; })()";
        let value = self
            .primitives
            .adapter()
            .evaluate_script_in_context(&context, script)
            .await
            .map_err(|err| {
                SoulBrowserError::internal(&format!("Failed to capture page snapshot: {}", err))
            })?;
        let url = value
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let title = value
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let text_sample = value
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(PageHealthSnapshot {
            url,
            title,
            text_sample,
        })
    }

    async fn evaluate_weather_candidate(
        &self,
        exec_ctx: &ExecCtx,
        candidate: &str,
        destination_selector: &str,
        timeout_ms: u64,
    ) -> Result<WeatherCandidateEvaluation, SoulBrowserError> {
        self.primitives
            .navigate(exec_ctx, candidate, WaitTier::DomReady)
            .await
            .map_err(|err| Self::map_action_error("Weather result navigate", err))?;
        self.primitives
            .wait_for(
                exec_ctx,
                &WaitCondition::ElementVisible(AnchorDescriptor::Css(
                    destination_selector.to_string(),
                )),
                timeout_ms,
            )
            .await
            .map_err(|err| Self::map_action_error("Weather destination wait", err))?;
        let snapshot = self.capture_page_snapshot(exec_ctx).await?;
        if snapshot.url.to_ascii_lowercase().contains("baidu.com") {
            return Ok(WeatherCandidateEvaluation::Blocked(
                "Baidu relay did not resolve to a weather domain".to_string(),
            ));
        }
        if let Some(reason) =
            detect_block_reason(&snapshot.title, &snapshot.text_sample, Some(&snapshot.url))
        {
            return Ok(WeatherCandidateEvaluation::Blocked(reason));
        }
        Ok(WeatherCandidateEvaluation::Ready(WeatherCandidateOutcome {
            final_url: snapshot.url.clone(),
            snapshot: snapshot.as_value(),
        }))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for BrowserToolExecutor {
    async fn execute(
        &self,
        context: ToolExecutionContext,
    ) -> Result<ToolExecutionResult, SoulBrowserError> {
        let start = std::time::Instant::now();
        let span = obs_tracing::tool_span(&context.tool_id);
        let _span_guard = span.enter();
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "tool_id".to_string(),
            serde_json::Value::String(context.tool_id.clone()),
        );

        let exec_ctx = self.build_exec_ctx(context.route.as_ref(), context.timeout_ms);

        match context.tool_id.as_str() {
            "navigate-to-url" | "browser.navigate" => {
                let url = context
                    .input
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        SoulBrowserError::validation_error(
                            "Missing field",
                            "'url' is required for navigate-to-url",
                        )
                    })?;
                let wait = context
                    .input
                    .get("wait_tier")
                    .map(|v| Self::parse_wait_tier(Some(v)))
                    .unwrap_or(WaitTier::Idle);
                let report = self
                    .primitives
                    .navigate(&exec_ctx, url, wait)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        Self::map_action_error("Navigate", err)
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "navigated",
                        "url": url,
                        "latency_ms": report.latency_ms,
                        "wait_tier": format!("{:?}", wait).to_lowercase(),
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "click" | "browser.click" => {
                let anchor_val = context
                    .input
                    .get("anchor")
                    .or_else(|| context.input.get("selector"));
                let anchor_json = anchor_val.ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'anchor' or 'selector' is required for click",
                    )
                })?;
                let anchor = Self::parse_anchor(anchor_json)?;
                let wait = context
                    .input
                    .get("wait_tier")
                    .map(|v| Self::parse_wait_tier(Some(v)))
                    .unwrap_or(WaitTier::DomReady);
                let report = self
                    .primitives
                    .click(&exec_ctx, &anchor, wait)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        Self::map_action_error("Click", err)
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "clicked",
                        "anchor": anchor.to_string(),
                        "latency_ms": report.latency_ms,
                        "wait_tier": format!("{:?}", wait).to_lowercase(),
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "type-text" | "browser.type" => {
                let anchor_val = context
                    .input
                    .get("anchor")
                    .or_else(|| context.input.get("selector"));
                let anchor_json = anchor_val.ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'anchor' or 'selector' is required for type-text",
                    )
                })?;
                let anchor = Self::parse_anchor(anchor_json)?;
                let text = context
                    .input
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        SoulBrowserError::validation_error(
                            "Missing field",
                            "'text' is required for type-text",
                        )
                    })?;
                let submit = context
                    .input
                    .get("submit")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let wait_tier = context
                    .input
                    .get("wait_tier")
                    .map(|v| Self::parse_wait_tier(Some(v)));
                let report = self
                    .primitives
                    .type_text(&exec_ctx, &anchor, text, submit, wait_tier)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        Self::map_action_error("Type text", err)
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "typed",
                        "anchor": anchor.to_string(),
                        "length": text.len(),
                        "submit": submit,
                        "latency_ms": report.latency_ms,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "select-option" | "browser.select" => {
                let anchor_val = context
                    .input
                    .get("anchor")
                    .or_else(|| context.input.get("selector"));
                let anchor_json = anchor_val.ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'anchor' or 'selector' is required for select-option",
                    )
                })?;
                let anchor = Self::parse_anchor(anchor_json)?;
                let value = context
                    .input
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        SoulBrowserError::validation_error(
                            "Missing field",
                            "'value' is required for select-option",
                        )
                    })?;
                let match_kind_value = context.input.get("match_kind").and_then(|v| v.as_str());
                let method = Self::parse_select_method(match_kind_value)?;
                let wait = context
                    .input
                    .get("wait_tier")
                    .map(|v| Self::parse_wait_tier(Some(v)))
                    .unwrap_or(WaitTier::DomReady);
                let report = self
                    .primitives
                    .select(&exec_ctx, &anchor, method, value, wait)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        Self::map_action_error("Select", err)
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "selected",
                        "anchor": anchor.to_string(),
                        "value": value,
                        "match_kind": match_kind_value,
                        "latency_ms": report.latency_ms,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "scroll-page" => {
                let target_json = context.input.get("target").ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'target' is required for scroll-page",
                    )
                })?;
                let target = Self::parse_scroll_target(target_json)?;
                let behavior = Self::parse_scroll_behavior(context.input.get("behavior"))?;
                let report = self
                    .primitives
                    .scroll(&exec_ctx, &target, behavior)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        Self::map_action_error("Scroll", err)
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "scrolled",
                        "target": format!("{:?}", target),
                        "behavior": format!("{:?}", behavior).to_lowercase(),
                        "latency_ms": report.latency_ms,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "wait-for-element" => {
                let target = context.input.get("target").ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'target' is required for wait-for-element",
                    )
                })?;
                let condition = context.input.get("condition").ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'condition' is required for wait-for-element",
                    )
                })?;
                let wait_condition = Self::parse_wait_condition_for_element(target, condition)?;
                let report = self
                    .primitives
                    .wait_for(&exec_ctx, &wait_condition, context.timeout_ms)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        Self::map_action_error("Wait for element", err)
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "condition_met",
                        "condition": format!("{:?}", wait_condition),
                        "latency_ms": report.latency_ms,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "wait-for-condition" => {
                let expect = context.input.get("expect").ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'expect' is required for wait-for-condition",
                    )
                })?;
                let wait_condition = Self::parse_wait_condition_for_expect(expect)?;
                let report = self
                    .primitives
                    .wait_for(&exec_ctx, &wait_condition, context.timeout_ms)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        Self::map_action_error("Wait for condition", err)
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "condition_met",
                        "condition": format!("{:?}", wait_condition),
                        "latency_ms": report.latency_ms,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "browser.search" => {
                let query = required_field(
                    &context.input,
                    "query",
                    "Missing field",
                    "'query' is required for browser.search",
                )?;
                let site_filter = context
                    .input
                    .get("site")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                let engine_hint = context
                    .input
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .unwrap_or("baidu");
                let override_url = context
                    .input
                    .get("search_url")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty());
                let engines = search_engine_attempts(engine_hint);
                let mut last_error: Option<SoulBrowserError> = None;
                for engine in engines {
                    let (engine_label, search_url) = build_browser_search_url(
                        engine,
                        &query,
                        site_filter.as_deref(),
                        override_url,
                    );
                    let selectors = collect_results_selectors(&context.input, &engine_label);
                    let selector_summary = selectors.join(", ");
                    let url_condition = search_url_wait_condition(&engine_label, &search_url);

                    let navigate_result = self
                        .primitives
                        .navigate(&exec_ctx, &search_url, WaitTier::DomReady)
                        .await;
                    if let Err(err) = navigate_result {
                        last_error = Some(Self::map_action_error("Browser search", err));
                        continue;
                    }

                    let mut wait_success = None;
                    let mut selector_error: Option<SoulBrowserError> = None;
                    // Use shorter timeout per selector to allow trying multiple selectors
                    let per_selector_timeout = (context.timeout_ms / selectors.len() as u64)
                        .max(3000)
                        .min(8000);
                    for selector in selectors.iter() {
                        let wait = self
                            .primitives
                            .wait_for(
                                &exec_ctx,
                                &WaitCondition::ElementVisible(AnchorDescriptor::Css(
                                    selector.clone(),
                                )),
                                per_selector_timeout,
                            )
                            .await;
                        match wait {
                            Ok(_) => {
                                wait_success = Some(selector.clone());
                                break;
                            }
                            Err(ActionError::WaitTimeout(_)) => {
                                selector_error = Some(SoulBrowserError::operation_failed(
                                    "browser.search",
                                    &format!(
                                        "Search results not detected on {engine} using selector {selector}",
                                        engine = engine_label,
                                        selector = selector,
                                    ),
                                ));
                                continue;
                            }
                            Err(err) => {
                                selector_error =
                                    Some(Self::map_action_error("Browser search wait", err));
                                continue;
                            }
                        }
                    }

                    if let Some(matched_selector) = wait_success {
                        metadata.insert("search_url".to_string(), json!(search_url.clone()));
                        metadata.insert("search_engine".to_string(), json!(engine_label.clone()));
                        metadata.insert(
                            "results_selector".to_string(),
                            json!(matched_selector.clone()),
                        );
                        let duration = start.elapsed().as_millis() as u64;
                        let result = ToolExecutionResult {
                            success: true,
                            output: Some(serde_json::json!({
                                "status": "search_ready",
                                "query": query,
                                "engine": engine_label,
                                "url": search_url,
                                "results_selector": matched_selector,
                            })),
                            error: None,
                            duration_ms: duration,
                            metadata,
                        };
                        let result = self.finish_tool(&context, start, &span, result);
                        return Ok(result);
                    } else {
                        match self
                            .primitives
                            .wait_for(&exec_ctx, &url_condition, context.timeout_ms)
                            .await
                        {
                            Ok(_) => {
                                last_error = selector_error.or_else(|| {
                                    Some(SoulBrowserError::operation_failed(
                                        "browser.search",
                                        &format!(
                                            "Search results not detected on {engine} (selectors: {selectors})",
                                            engine = engine_label,
                                            selectors = selector_summary,
                                        ),
                                    ))
                                });
                            }
                            Err(err) => {
                                last_error =
                                    Some(Self::map_action_error("Browser search wait", err));
                            }
                        }
                        continue;
                    }
                }

                let err = last_error.unwrap_or_else(|| {
                    SoulBrowserError::operation_failed(
                        "browser.search",
                        "Search results not detected",
                    )
                });
                self.record_error(&context, start, &span);
                Err(err)
            }
            "browser.search.click-result" => {
                let engine_hint = context
                    .input
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .unwrap_or("baidu");
                let engine_label = normalized_search_engine(engine_hint);
                let selectors = auto_act_candidate_selectors(&context.input, engine_label);
                if selectors.is_empty() {
                    self.record_error(&context, start, &span);
                    return Err(SoulBrowserError::validation_error(
                        "Missing selectors",
                        "'selectors' or known search selectors are required for browser.search.click-result",
                    ));
                }
                let domains = auto_act_domain_hints(&context.input);
                let max_candidates = context
                    .input
                    .get("max_candidates")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(40);
                let max_attempts = context
                    .input
                    .get("max_attempts")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(4) as usize;
                let wait = context
                    .input
                    .get("wait_tier")
                    .map(|v| Self::parse_wait_tier(Some(v)))
                    .unwrap_or(WaitTier::DomReady);
                let wait_per_candidate_ms = context
                    .input
                    .get("wait_per_candidate_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(15_000)
                    .max(1);
                let per_attempt_wait_ms = wait_per_candidate_ms.max(1_000);
                let mut excluded_urls: Vec<String> = Vec::new();
                if let Some(value) = context.input.get("exclude_urls") {
                    append_selector_value(value, &mut excluded_urls);
                }
                excluded_urls = excluded_urls
                    .into_iter()
                    .map(|url| url.trim().to_string())
                    .filter(|url| !url.is_empty())
                    .collect();
                let mut last_error: Option<SoulBrowserError> = None;
                let mut attempt_logs: Vec<Value> = Vec::new();
                let mut last_auto_act_failure: Option<(String, Vec<String>)> = None;
                let dom_context = match self.primitives.resolve_context(&exec_ctx).await {
                    Ok(ctx_handle) => ctx_handle,
                    Err(err) => {
                        self.record_error(&context, start, &span);
                        return Err(SoulBrowserError::internal(&format!(
                            "Failed to resolve execution context: {}",
                            err
                        )));
                    }
                };

                for attempt_index in 0..max_attempts {
                    let attempt_number = (attempt_index + 1) as u64;
                    let marker_value = format!("autoact-{}", Uuid::new_v4().simple());
                    let script_config = json!({
                        "engine": engine_label,
                        "selectors": selectors,
                        "domains": domains,
                        "marker_attr": SEARCH_RESULT_ATTR,
                        "marker_value": marker_value,
                        "max_candidates": max_candidates,
                        "exclude_urls": excluded_urls,
                    });
                    let script = auto_act_result_picker_script(&script_config)?;
                    let script_output = match self
                        .primitives
                        .adapter()
                        .evaluate_script_in_context(&dom_context, &script)
                        .await
                    {
                        Ok(value) => value,
                        Err(err) => {
                            self.record_error(&context, start, &span);
                            return Err(SoulBrowserError::internal(&format!(
                                "Failed to inspect search results: {}",
                                err
                            )));
                        }
                    };
                    let status = script_output
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if status != "target_marked" {
                        let reason = script_output
                            .get("reason")
                            .and_then(Value::as_str)
                            .unwrap_or("未能定位可点击的搜索结果");
                        last_error = Some(SoulBrowserError::operation_failed(
                            "browser.search.click-result",
                            reason,
                        ));
                        break;
                    }
                    let anchor_selector =
                        match script_output.get("anchor_selector").and_then(Value::as_str) {
                            Some(selector) => selector.to_string(),
                            None => {
                                self.record_error(&context, start, &span);
                                return Err(SoulBrowserError::internal(
                                    "AutoAct result picker did not return anchor_selector",
                                ));
                            }
                        };
                    let anchor = AnchorDescriptor::Css(anchor_selector.clone());
                    let report = self
                        .primitives
                        .click(&exec_ctx, &anchor, wait)
                        .await
                        .map_err(|err| {
                            self.record_error(&context, start, &span);
                            Self::map_action_error("Search result click", err)
                        })?;
                    let selected_url = script_output
                        .get("selected_url")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                    let matched_domain = script_output
                        .get("matched_domain")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                    let mut waited_pattern: Option<String> = None;
                    let patterns = post_click_patterns(
                        selected_url.as_deref(),
                        matched_domain.as_deref(),
                        &domains,
                        engine_label,
                    );
                    let mut wait_failed = false;
                    if !patterns.is_empty() {
                        for pattern in patterns.iter() {
                            let condition = WaitCondition::UrlMatches(pattern.clone());
                            match self
                                .primitives
                                .wait_for(
                                    &exec_ctx,
                                    &condition,
                                    dynamic_wait_timeout(
                                        pattern,
                                        &domains,
                                        engine_label,
                                        per_attempt_wait_ms,
                                    ),
                                )
                                .await
                            {
                                Ok(wait_report) if wait_report.ok => {
                                    waited_pattern = Some(pattern.clone());
                                    break;
                                }
                                Ok(_) => {
                                    wait_failed = true;
                                    continue;
                                }
                                Err(ActionError::WaitTimeout(_)) => {
                                    wait_failed = true;
                                    continue;
                                }
                                Err(err) => {
                                    self.record_error(&context, start, &span);
                                    return Err(Self::map_action_error(
                                        "Search result stabilization",
                                        err,
                                    ));
                                }
                            }
                        }
                        if waited_pattern.is_none() {
                            wait_failed = true;
                        }
                    }
                    let pattern_values: Vec<Value> = patterns
                        .iter()
                        .map(|pattern| Value::String(pattern.clone()))
                        .collect();
                    attempt_logs.push(json!({
                        "attempt_index": attempt_number,
                        "selected_url": selected_url.clone(),
                        "matched_domain": matched_domain.clone(),
                        "wait_patterns": pattern_values,
                        "wait_pattern": waited_pattern.clone(),
                        "wait_success": !wait_failed,
                    }));

                    if !wait_failed {
                        metadata.insert(
                            "auto_act_engine".to_string(),
                            Value::String(engine_label.to_string()),
                        );
                        metadata.insert(
                            "auto_act_attempt_index".to_string(),
                            Value::Number(attempt_number.into()),
                        );
                        let attempts_value = Value::Array(attempt_logs.clone());
                        metadata.insert("auto_act_attempts".to_string(), attempts_value.clone());
                        let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                        let result = ToolExecutionResult {
                            success: report.ok,
                            output: Some(serde_json::json!({
                                "status": "result_clicked",
                                "engine": engine_label,
                                "anchor": anchor_selector,
                                "target_url": script_output
                                    .get("selected_url")
                                    .and_then(Value::as_str),
                                "matched_domain": script_output
                                    .get("matched_domain")
                                    .and_then(Value::as_str),
                                "fallback_used": script_output
                                    .get("fallback_used")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false),
                                "candidate_count": script_output
                                    .get("candidate_count")
                                    .and_then(Value::as_u64),
                                "wait_pattern": waited_pattern,
                                "attempts": attempts_value,
                            })),
                            error: None,
                            duration_ms: duration,
                            metadata,
                        };
                        let result = self.finish_tool(&context, start, &span, result);
                        return Ok(result);
                    }

                    let failure_reason = match selected_url.as_deref() {
                        Some(url) => format!("未能加载预期站点：{}", url),
                        None => "未能加载预期站点".to_string(),
                    };
                    let coded_reason = format!("[auto_act_candidates_exhausted] {failure_reason}");
                    last_error = Some(SoulBrowserError::operation_failed(
                        "browser.search.click-result",
                        &coded_reason,
                    ));
                    if let Some(url) = selected_url {
                        if !excluded_urls.iter().any(|existing| existing == &url) {
                            excluded_urls.push(url);
                        }
                    }
                    last_auto_act_failure = Some((coded_reason.clone(), excluded_urls.clone()));
                }

                if let Some((coded_reason, excluded_snapshot)) = last_auto_act_failure {
                    metadata.insert(
                        "auto_act_engine".to_string(),
                        Value::String(engine_label.to_string()),
                    );
                    metadata.insert(
                        "auto_act_attempt_index".to_string(),
                        Value::Number((attempt_logs.len() as u64).into()),
                    );
                    let attempts_value = Value::Array(attempt_logs.clone());
                    metadata.insert("auto_act_attempts".to_string(), attempts_value.clone());
                    let excluded_value = Value::Array(
                        excluded_snapshot
                            .iter()
                            .map(|url| Value::String(url.clone()))
                            .collect(),
                    );
                    metadata.insert("auto_act_excluded_urls".to_string(), excluded_value.clone());
                    let duration = start.elapsed().as_millis() as u64;
                    let result = ToolExecutionResult {
                        success: false,
                        output: Some(serde_json::json!({
                            "status": "auto_act_candidates_exhausted",
                            "engine": engine_label,
                            "attempts": attempts_value,
                            "excluded_urls": excluded_value,
                        })),
                        error: Some(coded_reason),
                        duration_ms: duration,
                        metadata,
                    };
                    let result = self.finish_tool(&context, start, &span, result);
                    return Ok(result);
                }
                self.record_error(&context, start, &span);
                Err(last_error.unwrap_or_else(|| {
                    SoulBrowserError::operation_failed(
                        "browser.search.click-result",
                        "SERP 候选项全部尝试后仍未进入权威站点",
                    )
                }))
            }
            "browser.close-modal" => {
                let selectors = modal_selectors_from_input(&context.input);
                let wait_tier = context
                    .input
                    .get("wait_tier")
                    .map(|v| Self::parse_wait_tier(Some(v)))
                    .unwrap_or(WaitTier::DomReady);
                let mut clicked_selector: Option<String> = None;
                let mut last_error: Option<String> = None;

                for selector in selectors.iter() {
                    let anchor = AnchorDescriptor::Css(selector.clone());
                    match self.primitives.click(&exec_ctx, &anchor, wait_tier).await {
                        Ok(report) => {
                            if report.ok {
                                clicked_selector = Some(selector.clone());
                                break;
                            }
                        }
                        Err(err) => {
                            last_error = Some(err.to_string());
                            continue;
                        }
                    }
                }

                let fallback_escape = context
                    .input
                    .get("fallback_escape")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let focus_selector = context.input.get("focus_selector").and_then(|v| v.as_str());
                let mut escape_used = false;
                if clicked_selector.is_none() && fallback_escape {
                    escape_used = match self
                        .dispatch_keyboard_event(&exec_ctx, "Escape", "Escape", 27, focus_selector)
                        .await
                    {
                        Ok(dispatched) => dispatched,
                        Err(err) => {
                            self.record_error(&context, start, &span);
                            return Err(err);
                        }
                    };
                }

                if clicked_selector.is_none() && !escape_used {
                    self.record_error(&context, start, &span);
                    let detail =
                        last_error.unwrap_or_else(|| "No close controls matched".to_string());
                    return Err(SoulBrowserError::validation_error(
                        "未找到可关闭的弹窗",
                        &detail,
                    ));
                }

                let duration = start.elapsed().as_millis() as u64;
                let result = ToolExecutionResult {
                    success: true,
                    output: Some(serde_json::json!({
                        "status": "modal_closed",
                        "clicked_selector": clicked_selector,
                        "escape_used": escape_used,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "browser.send-esc" => {
                let focus_selector = context.input.get("focus_selector").and_then(|v| v.as_str());
                let count = context
                    .input
                    .get("count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1)
                    .clamp(1, 5) as usize;
                let mut dispatched = false;
                for _ in 0..count {
                    let emitted = match self
                        .dispatch_keyboard_event(&exec_ctx, "Escape", "Escape", 27, focus_selector)
                        .await
                    {
                        Ok(value) => value,
                        Err(err) => {
                            self.record_error(&context, start, &span);
                            return Err(err);
                        }
                    };
                    dispatched |= emitted;
                }
                let duration = start.elapsed().as_millis() as u64;
                let result = ToolExecutionResult {
                    success: true,
                    output: Some(serde_json::json!({
                        "status": "escape_dispatched",
                        "count": count,
                        "dispatched": dispatched,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "weather.search" => {
                let query = required_field(
                    &context.input,
                    "query",
                    "Missing field",
                    "'query' is required for weather.search",
                )?;
                let override_url = context.input.get("search_url").and_then(|v| v.as_str());
                let search_url = build_weather_search_url(&query, override_url);
                let result_selector = context
                    .input
                    .get("result_selector")
                    .and_then(|v| v.as_str())
                    .unwrap_or("div#content_left");
                let follow_link = context
                    .input
                    .get("follow_result_link")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let link_patterns = build_weather_link_patterns(&context.input);
                let preferred_override = context
                    .input
                    .get("preferred_link_substring")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                let destination_selector = context
                    .input
                    .get("destination_selector")
                    .and_then(|v| v.as_str())
                    .unwrap_or("body");
                let wait_condition = weather_wait_condition(&search_url);
                let widget_anchor = AnchorDescriptor::Css(result_selector.to_string());

                let nav_report = self
                    .primitives
                    .navigate(&exec_ctx, &search_url, WaitTier::DomReady)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        Self::map_action_error("Weather search", err)
                    })?;

                self.primitives
                    .wait_for(&exec_ctx, &wait_condition, context.timeout_ms)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        Self::map_action_error("Weather search wait", err)
                    })?;

                self.primitives
                    .wait_for(
                        &exec_ctx,
                        &WaitCondition::ElementVisible(widget_anchor.clone()),
                        context.timeout_ms,
                    )
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        Self::map_action_error("Weather widget wait", err)
                    })?;

                let mut destination_url = search_url.clone();
                let mut page_snapshot = Value::Null;
                if follow_link {
                    let candidates = self
                        .extract_weather_result_links(
                            &exec_ctx,
                            &link_patterns,
                            WEATHER_CANDIDATE_LIMIT,
                            preferred_override.as_deref(),
                        )
                        .await?;
                    let mut last_block_reason: Option<String> = None;
                    for candidate in candidates.iter() {
                        match self
                            .evaluate_weather_candidate(
                                &exec_ctx,
                                candidate,
                                destination_selector,
                                context.timeout_ms,
                            )
                            .await
                        {
                            Ok(WeatherCandidateEvaluation::Ready(outcome)) => {
                                destination_url = outcome.final_url;
                                page_snapshot = outcome.snapshot;
                                break;
                            }
                            Ok(WeatherCandidateEvaluation::Blocked(reason)) => {
                                warn!(target = %candidate, reason = %reason, "Weather candidate blocked");
                                last_block_reason = Some(reason);
                                continue;
                            }
                            Err(err) => {
                                self.record_error(&context, start, &span);
                                return Err(err);
                            }
                        }
                    }
                    if destination_url == search_url
                        && !candidates.is_empty()
                        && last_block_reason.is_some()
                    {
                        let reason = last_block_reason.unwrap();
                        return Err(SoulBrowserError::forbidden(
                            "天气站点被拦截，需人工验证或稍后重试",
                        )
                        .with_cause(WEATHER_BLOCKED_CAUSE, &reason));
                    }
                }

                metadata.insert(
                    "destination_url".to_string(),
                    json!(destination_url.clone()),
                );
                metadata.insert("current_url".to_string(), json!(destination_url.clone()));
                if !page_snapshot.is_null() {
                    metadata.insert("page_snapshot".to_string(), page_snapshot.clone());
                }

                let duration = start.elapsed().as_millis() as u64;
                let result = ToolExecutionResult {
                    success: true,
                    output: Some(serde_json::json!({
                        "status": "weather_ready",
                        "query": query,
                        "url": search_url,
                        "destination_url": destination_url,
                        "page_snapshot": page_snapshot,
                        "result_selector": result_selector,
                        "latency_ms": nav_report.latency_ms,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "get-element-info" => {
                let anchor_json = context.input.get("anchor").ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'anchor' is required for get-element-info",
                    )
                })?;
                let anchor = Self::parse_anchor(anchor_json)?;
                let include = context.input.get("include").cloned().unwrap_or_default();
                let result = ToolExecutionResult {
                    success: true,
                    output: Some(serde_json::json!({
                        "status": "inspected",
                        "anchor": anchor.to_string(),
                        "include": include,
                    })),
                    error: None,
                    duration_ms: start.elapsed().as_millis() as u64,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "retrieve-history" => {
                let limit = context
                    .input
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let result = ToolExecutionResult {
                    success: true,
                    output: Some(serde_json::json!({
                        "status": "history",
                        "events": serde_json::Value::Array(vec![]),
                        "limit": limit,
                    })),
                    error: None,
                    duration_ms: start.elapsed().as_millis() as u64,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "take-screenshot" | "browser.screenshot" => {
                let route = match context.route.as_ref() {
                    Some(route) => route,
                    None => {
                        let duration = start.elapsed().as_millis() as u64;
                        metadata.insert("reason".to_string(), serde_json::json!("missing route"));
                        let result = ToolExecutionResult {
                            success: false,
                            output: Some(serde_json::json!({
                                "status": "failed",
                                "reason": "execution route required",
                            })),
                            error: Some(
                                "take-screenshot requires a page/frame execution route".to_string(),
                            ),
                            duration_ms: duration,
                            metadata,
                        };
                        let result = self.finish_tool(&context, start, &span, result);
                        return Ok(result);
                    }
                };

                let page_id = match self.resolve_page_for_route(route).await {
                    Ok(id) => id,
                    Err(err) => {
                        let duration = start.elapsed().as_millis() as u64;
                        metadata.insert("reason".to_string(), serde_json::json!("invalid route"));
                        let result = ToolExecutionResult {
                            success: false,
                            output: Some(serde_json::json!({
                                "status": "failed",
                                "reason": err.dev_message().unwrap_or("invalid route"),
                            })),
                            error: Some(err.to_string()),
                            duration_ms: duration,
                            metadata,
                        };
                        let result = self.finish_tool(&context, start, &span, result);
                        return Ok(result);
                    }
                };

                if let Err(err) = self.ensure_adapter_started().await {
                    let duration = start.elapsed().as_millis() as u64;
                    metadata.insert(
                        "reason".to_string(),
                        serde_json::json!("adapter start failed"),
                    );
                    let result = ToolExecutionResult {
                        success: false,
                        output: Some(serde_json::json!({
                            "status": "failed",
                            "reason": err.dev_message().unwrap_or("adapter unavailable"),
                        })),
                        error: Some(err.to_string()),
                        duration_ms: duration,
                        metadata,
                    };
                    let result = self.finish_tool(&context, start, &span, result);
                    return Ok(result);
                }

                let remaining = exec_ctx.remaining_time();
                let deadline = if remaining.is_zero() {
                    Duration::from_secs(30)
                } else {
                    remaining
                };

                let session_id = route.session.0.clone();
                let page_id_str = route.page.0.clone();
                let frame_id = route.frame.0.clone();

                match self.adapter.screenshot(page_id, deadline).await {
                    Ok(bytes) => {
                        let duration = start.elapsed().as_millis() as u64;
                        let byte_len = bytes.len() as u64;
                        metadata.insert(
                            "page_id".to_string(),
                            serde_json::json!(page_id_str.clone()),
                        );
                        metadata.insert("byte_len".to_string(), serde_json::json!(byte_len));
                        metadata.insert("content_type".to_string(), serde_json::json!("image/png"));

                        let mut output = serde_json::json!({
                            "status": "captured",
                            "byte_len": byte_len,
                            "content_type": "image/png",
                            "route": {
                                "session": session_id,
                                "page": page_id_str,
                                "frame": frame_id,
                            },
                            "bytes": bytes,
                        });

                        if let Some(filename) =
                            context.input.get("filename").and_then(|v| v.as_str())
                        {
                            if let Some(obj) = output.as_object_mut() {
                                obj.insert("filename".to_string(), serde_json::json!(filename));
                            }
                        }

                        let result = ToolExecutionResult {
                            success: true,
                            output: Some(output),
                            error: None,
                            duration_ms: duration,
                            metadata,
                        };
                        let result = self.finish_tool(&context, start, &span, result);
                        Ok(result)
                    }
                    Err(err) => {
                        let duration = start.elapsed().as_millis() as u64;
                        let hint = err.hint.clone();
                        let message = err.to_string();
                        metadata.insert("adapter_error".to_string(), serde_json::json!(message));
                        let result = ToolExecutionResult {
                            success: false,
                            output: Some(serde_json::json!({
                                "status": "failed",
                                "reason": hint.unwrap_or_else(|| "cdp adapter error".to_string()),
                            })),
                            error: Some(format!("Screenshot failed: {}", err)),
                            duration_ms: duration,
                            metadata,
                        };
                        let result = self.finish_tool(&context, start, &span, result);
                        Ok(result)
                    }
                }
            }
            "manual.pointer" => {
                let route = match context.route.as_ref() {
                    Some(route) => route,
                    None => {
                        let duration = start.elapsed().as_millis() as u64;
                        let result = ToolExecutionResult {
                            success: false,
                            output: None,
                            error: Some("manual.pointer requires an execution route".to_string()),
                            duration_ms: duration,
                            metadata,
                        };
                        let result = self.finish_tool(&context, start, &span, result);
                        return Ok(result);
                    }
                };

                let action = context
                    .input
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("click")
                    .to_ascii_lowercase();
                let x = context
                    .input
                    .get("x")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let y = context
                    .input
                    .get("y")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let outcome = match action.as_str() {
                    "click" => {
                        let button = context
                            .input
                            .get("button")
                            .and_then(|v| v.as_str())
                            .unwrap_or("left");
                        self.pointer_click(route, x, y, button).await
                    }
                    "scroll" => {
                        let delta_x = context
                            .input
                            .get("delta_x")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        let delta_y = context
                            .input
                            .get("delta_y")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        self.pointer_scroll(route, x, y, delta_x, delta_y).await
                    }
                    "type" => {
                        let Some(text) = context.input.get("text").and_then(|v| v.as_str()) else {
                            return Ok(self.finish_tool(
                                &context,
                                start,
                                &span,
                                ToolExecutionResult {
                                    success: false,
                                    output: None,
                                    error: Some("typing requires 'text'".to_string()),
                                    duration_ms: start.elapsed().as_millis() as u64,
                                    metadata,
                                },
                            ));
                        };
                        self.pointer_type(route, text).await
                    }
                    other => Err(SoulBrowserError::validation_error(
                        "Unsupported pointer action",
                        &format!("action '{}' is not supported", other),
                    )),
                };

                let (success, error_msg) = match outcome {
                    Ok(_) => (true, None),
                    Err(err) => (false, Some(err.to_string())),
                };

                let result = ToolExecutionResult {
                    success,
                    output: Some(serde_json::json!({
                        "status": if success { "dispatched" } else { "failed" },
                        "action": action,
                        "x": x,
                        "y": y,
                    })),
                    error: error_msg,
                    duration_ms: start.elapsed().as_millis() as u64,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "complete-task" => {
                let task_id = context
                    .input
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        SoulBrowserError::validation_error(
                            "Missing field",
                            "'task_id' is required for complete-task",
                        )
                    })?;
                let outcome = context
                    .input
                    .get("outcome")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let summary = context
                    .input
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let result = ToolExecutionResult {
                    success: true,
                    output: Some(serde_json::json!({
                        "status": "recorded",
                        "task_id": task_id,
                        "outcome": outcome,
                        "summary_len": summary.len(),
                    })),
                    error: None,
                    duration_ms: start.elapsed().as_millis() as u64,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "report-insight" => {
                let insight = context
                    .input
                    .get("insight")
                    .or_else(|| context.input.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let result = ToolExecutionResult {
                    success: true,
                    output: Some(serde_json::json!({
                        "status": "recorded",
                        "insight_len": insight.len(),
                    })),
                    error: None,
                    duration_ms: start.elapsed().as_millis() as u64,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "data.extract-site" => {
                return self
                    .execute_observation_tool(&context, start, &span, exec_ctx)
                    .await;
            }
            "market.quote.fetch" => {
                return self
                    .execute_quote_fetch_tool(&context, start, &span, exec_ctx)
                    .await;
            }
            tool if tool.starts_with("data.parse.") => {
                return self.execute_parse_tool(&context, start, &span, tool).await;
            }
            "data.validate.metal_price" => {
                return self
                    .execute_metal_price_validation(&context, start, &span)
                    .await;
            }
            "data.validate-target" => {
                return self.execute_target_validation(&context, start, &span).await;
            }
            "data.deliver.structured" => {
                return self.execute_deliver_tool(&context, start, &span).await;
            }
            other => {
                let result = ToolExecutionResult {
                    success: false,
                    output: None,
                    error: Some(format!("Unknown tool: {}", other)),
                    duration_ms: start.elapsed().as_millis() as u64,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
        }
    }

    fn cdp_adapter(&self) -> Option<Arc<CdpAdapter>> {
        Some(self.adapter.clone())
    }
}

fn parse_generic_observation(observation: &Value, schema: &str) -> Value {
    let data = observation.get("data").unwrap_or(observation);
    let url = data.get("url").and_then(|v| v.as_str());
    let title = data.get("title").and_then(|v| v.as_str());
    let text_sample = data
        .get("text_sample")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let summary: String = text_sample.chars().take(480).collect();

    json!({
        "schema": schema,
        "source_url": url,
        "captured_at": data.get("fetched_at").and_then(|v| v.as_str()),
        "title": title,
        "summary": summary,
        "text_sample": text_sample,
    })
}

fn canonical_observation<'a>(value: &'a Value) -> &'a Value {
    value.get("data").unwrap_or(value)
}

fn normalize_keywords(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if out
            .iter()
            .any(|existing: &String| existing.as_str().eq_ignore_ascii_case(trimmed))
        {
            continue;
        }
        out.push(trimmed.to_string());
    }
    out
}

fn normalize_domains(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        if let Some(domain) = normalize_domain_input(value) {
            if !out.iter().any(|existing| existing == &domain) {
                out.push(domain);
            }
        }
    }
    out
}

fn normalize_domain_input(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains("://") {
        Url::parse(trimmed)
            .ok()
            .and_then(|parsed| parsed.domain().map(|domain| domain.to_ascii_lowercase()))
    } else {
        Some(trimmed.trim_start_matches("www.").to_ascii_lowercase())
    }
}

fn domain_matches_allowed(actual: &str, allowed: &str) -> bool {
    if actual == allowed {
        return true;
    }
    actual
        .strip_suffix(allowed)
        .map(|prefix| prefix.ends_with('.'))
        .unwrap_or(false)
}

fn observation_text_blob(value: &Value) -> String {
    let mut parts = Vec::new();
    for key in [
        "title",
        "identity",
        "hero_text",
        "description",
        "text_sample",
    ] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            if !text.trim().is_empty() {
                parts.push(text.trim().to_string());
            }
        }
    }
    if let Some(meta) = value.get("meta").and_then(Value::as_object) {
        for entry in meta.values() {
            if let Some(text) = entry.as_str() {
                if !text.trim().is_empty() {
                    parts.push(text.trim().to_string());
                }
            }
        }
    }
    if let Some(headings) = value.get("headings").and_then(Value::as_array) {
        for heading in headings {
            if let Some(text) = heading.get("text").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    parts.push(text.trim().to_string());
                }
            }
        }
    }
    parts.join(" ").to_ascii_lowercase()
}

fn keyword_matches(haystack: &str, keyword: &str) -> bool {
    if keyword.trim().is_empty() {
        return false;
    }
    let lower = keyword.to_ascii_lowercase();
    haystack.contains(&lower)
}

fn extract_status_code(value: &Value) -> Option<u16> {
    for key in ["status_code", "status", "http_status"] {
        if let Some(code) = value.get(key).and_then(Value::as_i64) {
            return Some(code as u16);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::{response::Html, routing::get, Router};
    use cdp_adapter::{
        event_bus, AdapterError, AdapterErrorKind, CdpAdapter, CdpConfig, CdpTransport,
        CommandTarget, PageId as AdapterPageId, SessionId as AdapterSessionId, TransportEvent,
    };
    use serde_json::json;
    use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId as RouteSessionId};
    use std::{env, sync::Arc};
    use tokio::net::TcpListener;
    use tokio::sync::{Mutex, OnceCell};
    use uuid::Uuid;

    static FIXTURE_SERVER: OnceCell<String> = OnceCell::const_new();
    const TOOL_FIXTURE_HTML: &str = r#"<!DOCTYPE html>
<html lang='zh-CN'>
  <head>
    <meta charset='utf-8'>
    <title>Tool Fixture</title>
  </head>
  <body>
    <div id='app'>Fixture ready</div>
    <form>
      <label for='country'>Country</label>
      <select id='country' name='country'>
        <option value='cn'>China</option>
        <option value='us'>United States</option>
        <option value='jp'>Japan</option>
      </select>
    </form>
  </body>
</html>"#;

    async fn ensure_fixture_url() -> String {
        if let Ok(url) = env::var("SOULBROWSER_TOOL_FIXTURE_URL") {
            return url;
        }
        FIXTURE_SERVER
            .get_or_init(|| async {
                let router = Router::new()
                    .route("/", get(serve_fixture))
                    .route("/tool_fixture.html", get(serve_fixture));
                let listener = TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind fixture listener");
                let addr = listener.local_addr().expect("fixture addr");
                tokio::spawn(async move {
                    if let Err(err) = axum::serve(listener, router).await {
                        tracing::warn!(target = "tests", ?err, "fixture server stopped");
                    }
                });
                format!("http://{addr}/tool_fixture.html")
            })
            .await
            .clone()
    }

    async fn serve_fixture() -> Html<&'static str> {
        Html(TOOL_FIXTURE_HTML)
    }

    fn test_route() -> ExecRoute {
        ExecRoute::new(
            RouteSessionId(Uuid::new_v4().to_string()),
            PageId(Uuid::new_v4().to_string()),
            FrameId(Uuid::new_v4().to_string()),
        )
    }

    async fn navigate_to_fixture(executor: &BrowserToolExecutor, route: &ExecRoute, url: &str) {
        let context = ToolExecutionContext {
            tool_id: "navigate-to-url".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({"url": url}),
            timeout_ms: 5_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: Some(route.clone()),
        };
        executor.execute(context).await.unwrap();
    }

    #[tokio::test]
    async fn test_tool_manager() {
        let manager = BrowserToolManager::new("test-tenant".to_string());

        // Register all default tools (including legacy aliases)
        manager.register_default_tools().await.unwrap();

        // List tools
        let tools = manager.list_tools(None).await.unwrap();
        assert!(tools.len() >= 12);

        // Get specific tool
        let tool = manager.get_tool("navigate-to-url").await.unwrap();
        assert!(tool.is_some());

        // Execute tool
        let result = manager
            .execute(
                "navigate-to-url",
                "test-user",
                serde_json::json!({"url": "https://example.com"}),
            )
            .await
            .unwrap();
        assert!(result["success"].as_bool().unwrap());
        assert_eq!(
            result["metadata"]["tool_id"].as_str().unwrap(),
            "navigate-to-url"
        );
    }

    #[test]
    fn wait_condition_accepts_url_equals_literal() {
        let expect = json!({ "url_equals": "https://example.com/search?q=test" });
        let condition =
            BrowserToolExecutor::parse_wait_condition_for_expect(&expect).expect("condition");
        match condition {
            WaitCondition::UrlEquals(value) => {
                assert_eq!(value, "https://example.com/search?q=test")
            }
            other => panic!("unexpected condition: {other:?}"),
        }
    }

    #[test]
    fn post_click_patterns_prefer_guardrail_domains_on_baidu_redirects() {
        let patterns = super::post_click_patterns(
            Some("https://www.baidu.com/link?url=https%3A%2F%2Fexample.com%2Finfo"),
            None,
            &vec!["example.com".to_string()],
            "baidu",
        );
        assert!(patterns
            .first()
            .expect("patterns present")
            .contains(r"example\.com"));
        assert!(patterns
            .iter()
            .any(|pattern| pattern.contains(r"baidu\.com")));
    }

    #[tokio::test]
    async fn test_tool_executor() {
        let executor = BrowserToolExecutor::new();
        let fixture_url = ensure_fixture_url().await;
        let route = test_route();
        navigate_to_fixture(&executor, &route, &fixture_url).await;

        let context = ToolExecutionContext {
            tool_id: "navigate-to-url".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({"url": fixture_url}),
            timeout_ms: 5000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: Some(route.clone()),
        };

        let result = executor.execute(context).await.unwrap();
        assert!(result.success);
        assert!(result.output.is_some());

        let select_ctx = ToolExecutionContext {
            tool_id: "select-option".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({
                "selector": "#country",
                "value": "us",
                "match_kind": "value"
            }),
            timeout_ms: 5000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: Some(route.clone()),
        };

        let select_result = executor.execute(select_ctx).await.unwrap();
        assert!(select_result.success);
        assert!(select_result.output.is_some());
    }

    #[tokio::test]
    async fn test_take_screenshot_uses_adapter_bytes() {
        #[derive(Clone)]
        struct ScreenshotTransport {
            data: Arc<Mutex<Vec<(CommandTarget, String)>>>,
            screenshot_b64: String,
        }

        #[async_trait]
        impl CdpTransport for ScreenshotTransport {
            async fn start(&self) -> Result<(), AdapterError> {
                Ok(())
            }

            async fn next_event(&self) -> Option<TransportEvent> {
                None
            }

            async fn send_command(
                &self,
                target: CommandTarget,
                method: &str,
                _params: serde_json::Value,
            ) -> Result<serde_json::Value, AdapterError> {
                self.data
                    .lock()
                    .await
                    .push((target.clone(), method.to_string()));

                match method {
                    "Target.setDiscoverTargets" | "Target.setAutoAttach" => Ok(json!({})),
                    "Page.captureScreenshot" => Ok(json!({ "data": self.screenshot_b64 })),
                    other => Err(AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint(format!("unexpected method: {}", other))),
                }
            }
        }

        let (bus, _rx) = event_bus(8);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transport = ScreenshotTransport {
            data: calls.clone(),
            screenshot_b64: "iVBORw0KGgo=".to_string(),
        };

        let adapter = Arc::new(CdpAdapter::with_transport(
            CdpConfig::default(),
            bus,
            Arc::new(transport),
        ));

        let executor = BrowserToolExecutor {
            adapter: adapter.clone(),
            primitives: Arc::new(DefaultActionPrimitives::new(
                adapter.clone(),
                Arc::new(DefaultWaitStrategy::default()),
            )),
            policy_view: Arc::new(PolicyView::from(default_snapshot())),
            adapter_ready: OnceCell::new(),
            route_pages: DashMap::new(),
            page_routes: DashMap::new(),
            pending_pages: DashMap::new(),
        };

        let page_uuid = Uuid::new_v4();
        let session_uuid = Uuid::new_v4();
        adapter.register_page(
            AdapterPageId(page_uuid),
            AdapterSessionId(session_uuid),
            None,
            Some("mock-session".to_string()),
        );

        let frame_uuid = Uuid::new_v4().to_string();
        let route = ExecRoute::new(
            SessionId(session_uuid.to_string()),
            PageId(page_uuid.to_string()),
            FrameId(frame_uuid.clone()),
        );

        let context = ToolExecutionContext {
            tool_id: "take-screenshot".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({"filename": "mock.png"}),
            timeout_ms: 5_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: Some(route),
        };

        let result = executor.execute(context).await.unwrap();
        assert!(result.success, "expected screenshot tool to succeed");
        let output = result.output.expect("missing output payload");
        assert_eq!(output["byte_len"].as_u64(), Some(8));
        let bytes = output["bytes"].as_array().expect("bytes array");
        assert_eq!(bytes.len(), 8);
        assert_eq!(output["filename"].as_str(), Some("mock.png"));
        {
            let recorded = calls.lock().await;
            assert!(recorded
                .iter()
                .any(|(_, method)| method == "Page.captureScreenshot"));
        }
        assert_eq!(executor.route_pages.len(), 1);
        assert!(executor.pending_pages.is_empty());

        let second_route = ExecRoute::new(
            SessionId(session_uuid.to_string()),
            PageId(page_uuid.to_string()),
            FrameId(frame_uuid),
        );
        let second_context = ToolExecutionContext {
            tool_id: "take-screenshot".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({"filename": "second.png"}),
            timeout_ms: 5_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: Some(second_route),
        };

        let second_result = executor.execute(second_context).await.unwrap();
        assert!(second_result.success);

        let recorded = calls.lock().await;
        let create_count = recorded
            .iter()
            .filter(|(_, method)| method == "Target.createTarget")
            .count();
        assert_eq!(create_count, 0, "noop transport should not create targets");
        assert_eq!(
            executor.route_pages.len(),
            1,
            "cached route should be reused"
        );
        assert!(
            executor.pending_pages.is_empty(),
            "pending map should be clear"
        );
    }

    #[tokio::test]
    async fn test_scroll_tool_succeeds() {
        let executor = BrowserToolExecutor::new();

        let context = ToolExecutionContext {
            tool_id: "scroll-page".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({
                "target": { "kind": "top" },
                "behavior": "instant"
            }),
            timeout_ms: 2_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: None,
        };

        let result = executor.execute(context).await.unwrap();
        assert!(result.success);
        let output = result.output.expect("missing scroll output");
        assert_eq!(output["status"].as_str(), Some("scrolled"));
    }

    #[tokio::test]
    async fn test_wait_tools_produce_reports() {
        let executor = BrowserToolExecutor::new();
        let fixture_url = ensure_fixture_url().await;
        let route = test_route();
        navigate_to_fixture(&executor, &route, &fixture_url).await;

        let element_wait = ToolExecutionContext {
            tool_id: "wait-for-element".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({
                "target": { "anchor": "#app" },
                "condition": { "kind": "visible" },
                "timeout_ms": 500,
            }),
            timeout_ms: 2_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: Some(route.clone()),
        };

        let element_result = executor.execute(element_wait).await.unwrap();
        assert!(element_result.success);
        assert_eq!(
            element_result.output.as_ref().unwrap()["status"].as_str(),
            Some("condition_met")
        );

        let condition_wait = ToolExecutionContext {
            tool_id: "wait-for-condition".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({
                "expect": { "duration_ms": 100 },
                "timeout_ms": 500,
            }),
            timeout_ms: 1_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: Some(route.clone()),
        };

        let condition_result = executor.execute(condition_wait).await.unwrap();
        assert!(condition_result.success);
        assert_eq!(
            condition_result.output.as_ref().unwrap()["status"].as_str(),
            Some("condition_met")
        );
    }

    #[tokio::test]
    async fn test_retrieve_history_returns_payload() {
        let executor = BrowserToolExecutor::new();

        let context = ToolExecutionContext {
            tool_id: "retrieve-history".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({ "limit": 3 }),
            timeout_ms: 1_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: None,
        };

        let result = executor.execute(context).await.unwrap();
        assert!(result.success);
        let output = result.output.expect("missing retrieve-history output");
        assert_eq!(output["status"].as_str(), Some("history"));
        assert_eq!(output["limit"].as_u64(), Some(3));
    }
    #[test]
    fn convert_eastmoney_api_response() {
        let payload = json!({
            "data": {
                "diff": [
                    {"f14": "沪铜主连", "f2": 60590, "f4": -120, "f3": -0.2},
                    {"f14": "伦铜", "f2": 8450, "f4": 15, "f3": 0.18}
                ]
            }
        });
        let cfg = ApiQuoteConfig {
            url: "https://example.com".to_string(),
            ..Default::default()
        };
        let (tables, key_values) =
            BrowserToolExecutor::convert_api_response(&payload, &cfg, Some(10));
        assert_eq!(tables.len(), 1);
        assert_eq!(key_values.len(), 1);
        assert_eq!(key_values[0]["label"], "沪铜主连");
    }
}
