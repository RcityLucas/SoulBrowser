use crate::model::AgentRequest;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tracing::warn;

const DEFAULT_CONFIG_ENV: &str = "SOULBROWSER_QUOTE_SOURCES";
const DEFAULT_TTL: Duration = Duration::from_secs(300);
static REGISTRY: Lazy<QuoteSourceRegistry> =
    Lazy::new(|| match QuoteSourceRegistry::load_from_default() {
        Ok(registry) => registry,
        Err(err) => {
            warn!(
                target = "quotes",
                ?err,
                "failed to load quote source config, falling back to legacy defaults"
            );
            QuoteSourceRegistry::legacy()
        }
    });

#[derive(Debug, Clone)]
pub(crate) struct QuoteSourcePlan {
    pub navigate_url: String,
    pub payload: Value,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct QuoteQuery<'a> {
    pub metal_label: &'a str,
    pub contract: &'a str,
    pub slug: &'a str,
    pub prefer_spot: bool,
    pub allowed_markets: &'a [String],
}

pub(crate) fn resolve_quote_plan(request: &AgentRequest, query: QuoteQuery<'_>) -> QuoteSourcePlan {
    let mut explicit = if request.intent.target_sites_are_hints {
        None
    } else {
        request
            .intent
            .target_sites
            .first()
            .map(|site| site.trim())
            .filter(|site| !site.is_empty())
            .map(|site| site.to_string())
    };

    if let Some(site) = explicit.as_ref() {
        if !explicit_matches_query(site, query) {
            explicit = None;
        }
    }

    REGISTRY.plan_for(query, explicit.as_deref())
}

pub fn mark_source_unhealthy(source_id: &str, reason: &str) {
    REGISTRY.mark_unhealthy(source_id, reason);
}

#[derive(Debug, Clone, Deserialize)]
struct QuoteSourceFile {
    markets: Vec<MarketSourceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct MarketSourceConfig {
    id: String,
    label: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    sources: Vec<QuoteSourceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct QuoteSourceConfig {
    id: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    markets: Vec<String>,
    #[serde(default)]
    url_template: Option<String>,
    #[serde(default)]
    table_selectors: Vec<String>,
    #[serde(default)]
    key_value_selectors: Vec<SelectorConfig>,
    #[serde(default)]
    max_rows: Option<usize>,
    #[serde(default)]
    api: Option<ApiConfig>,
    #[serde(default)]
    probe: Option<ProbeConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub(crate) struct SelectorConfig {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub attribute: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub(crate) struct ApiFieldMapping {
    pub column: String,
    pub path: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub(crate) struct ApiConfig {
    pub url: String,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<Map<String, Value>>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub record_path: Option<Vec<String>>,
    #[serde(default)]
    pub field_mappings: Option<Vec<ApiFieldMapping>>,
    #[serde(default)]
    pub label_field: Option<String>,
    #[serde(default)]
    pub price_field: Option<String>,
    #[serde(default)]
    pub change_field: Option<String>,
    #[serde(default)]
    pub change_pct_field: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ProbeConfig {
    #[serde(default)]
    bad_patterns: Vec<String>,
    #[serde(default)]
    deny_status: Vec<u16>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct QuoteFetchSourcePayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    market: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    table_selectors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    key_value_selectors: Vec<SelectorConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_rows: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api: Option<ApiConfig>,
}

impl QuoteFetchSourcePayload {
    fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }
}

struct SourceHealth {
    last_checked: Option<Instant>,
    healthy: bool,
    message: Option<String>,
}

struct QuoteSourceRegistry {
    markets: Vec<MarketSourceConfig>,
    health: RwLock<HashMap<String, SourceHealth>>,
    ttl: Duration,
}

impl QuoteSourceRegistry {
    fn load_from_default() -> Result<Self, anyhow::Error> {
        let path = env::var(DEFAULT_CONFIG_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|_| Self::default_path());
        Self::load_from_path(&path)
    }

    fn default_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/defaults/quote_sources.yaml")
    }

    fn load_from_path(path: &PathBuf) -> Result<Self, anyhow::Error> {
        let content = fs::read_to_string(path)?;
        let parsed: QuoteSourceFile = serde_yaml::from_str(&content)?;
        let registry = Self {
            markets: parsed.markets,
            health: RwLock::new(HashMap::new()),
            ttl: DEFAULT_TTL,
        };
        registry.refresh_health(true);
        Ok(registry)
    }

    fn legacy() -> Self {
        Self {
            markets: Vec::new(),
            health: RwLock::new(HashMap::new()),
            ttl: DEFAULT_TTL,
        }
    }

    fn plan_for(&self, query: QuoteQuery<'_>, explicit_url: Option<&str>) -> QuoteSourcePlan {
        self.refresh_health(false);
        let mut attempts = if let Some(url) = explicit_url {
            let mut list = Vec::new();
            list.push(self.manual_override(url, query));
            list.extend(self.collect_candidates(query));
            dedup_payloads(list)
        } else {
            self.collect_candidates(query)
        };

        if attempts.is_empty() {
            attempts.push(self.legacy_payload(query, None));
        }

        let mut sources = Vec::new();
        let mut iter = attempts.into_iter();
        let mut primary = iter
            .next()
            .unwrap_or_else(|| self.legacy_payload(query, explicit_url));
        let fallback_values: Vec<Value> = iter
            .map(|payload| {
                if let Some(id) = payload.source_id.as_deref() {
                    sources.push(id.to_string());
                }
                payload.to_value()
            })
            .collect();
        if let Some(id) = primary.source_id.as_deref() {
            sources.insert(0, id.to_string());
        }
        primary.mode = Some("auto".to_string());
        let mut root = primary.to_value();
        root["fallback_sources"] = Value::Array(fallback_values);
        let navigate_url = root
            .get("source_url")
            .and_then(Value::as_str)
            .unwrap_or_else(|| explicit_url.unwrap_or("https://quote.eastmoney.com"))
            .to_string();
        QuoteSourcePlan {
            navigate_url,
            payload: root,
            sources,
        }
    }

    fn collect_candidates(&self, query: QuoteQuery<'_>) -> Vec<QuoteFetchSourcePayload> {
        let mut preferred: Vec<&QuoteSourceConfig> = Vec::new();
        let mut fallback: Vec<&QuoteSourceConfig> = Vec::new();
        let mut target_labels: HashSet<&str> =
            query.allowed_markets.iter().map(String::as_str).collect();
        if target_labels.is_empty() {
            target_labels.insert("上海期货交易所");
            target_labels.insert("伦敦金属交易所");
        }

        for market in &self.markets {
            if target_labels.contains(market.label.as_str())
                || market
                    .aliases
                    .iter()
                    .any(|alias| target_labels.contains(alias.as_str()))
            {
                for source in &market.sources {
                    if query.prefer_spot && source.tags.iter().any(|tag| tag == "spot") {
                        preferred.push(source);
                    } else {
                        fallback.push(source);
                    }
                }
            }
        }

        if preferred.is_empty() && fallback.is_empty() {
            for market in &self.markets {
                for source in &market.sources {
                    fallback.push(source);
                }
            }
        }

        let mut ordered: Vec<&QuoteSourceConfig> = if preferred.is_empty() {
            fallback
        } else {
            preferred.extend(fallback);
            preferred
        };

        ordered.sort_by(|a, b| {
            let score_a = (self.health_score(a), source_priority(a, query.prefer_spot));
            let score_b = (self.health_score(b), source_priority(b, query.prefer_spot));
            score_b.cmp(&score_a)
        });
        let mut seen = HashSet::new();
        ordered
            .into_iter()
            .filter_map(|source| {
                if !seen.insert(source.id.clone()) {
                    return None;
                }
                self.render_source(source, query)
            })
            .collect()
    }

    fn render_source(
        &self,
        source: &QuoteSourceConfig,
        query: QuoteQuery<'_>,
    ) -> Option<QuoteFetchSourcePayload> {
        let url = source
            .url_template
            .as_ref()
            .map(|template| apply_template(template, query));
        if url.is_none() && source.api.is_none() {
            return None;
        }
        let mut selectors: Vec<SelectorConfig> = Vec::new();
        for selector in &source.key_value_selectors {
            let mut rendered = selector.clone();
            if let Some(label) = selector.label.as_ref() {
                rendered.label = Some(apply_template(label, query));
            }
            selectors.push(rendered);
        }
        let api = source.api.as_ref().map(|cfg| cfg.render(query));
        Some(QuoteFetchSourcePayload {
            source_id: Some(source.id.clone()),
            market: source
                .markets
                .first()
                .cloned()
                .or_else(|| Some("market".to_string())),
            mode: None,
            source_url: url.clone(),
            table_selectors: if source.table_selectors.is_empty() {
                default_table_selectors()
            } else {
                source.table_selectors.clone()
            },
            key_value_selectors: if selectors.is_empty() {
                default_key_selectors(query)
            } else {
                selectors
            },
            max_rows: source.max_rows.or(Some(50)),
            api,
        })
    }

    fn manual_override(&self, url: &str, query: QuoteQuery<'_>) -> QuoteFetchSourcePayload {
        QuoteFetchSourcePayload {
            source_id: Some("manual_override".to_string()),
            market: query.allowed_markets.first().cloned(),
            mode: None,
            source_url: Some(url.to_string()),
            table_selectors: default_table_selectors(),
            key_value_selectors: default_key_selectors(query),
            max_rows: Some(50),
            api: None,
        }
    }

    fn legacy_payload(
        &self,
        query: QuoteQuery<'_>,
        explicit_url: Option<&str>,
    ) -> QuoteFetchSourcePayload {
        let default_url = if query.prefer_spot {
            format!("https://data.eastmoney.com/metal/{}.html", query.slug)
        } else {
            format!(
                "https://quote.eastmoney.com/qh/{}.html",
                query.contract.to_ascii_uppercase()
            )
        };
        QuoteFetchSourcePayload {
            source_id: Some("legacy".to_string()),
            market: query.allowed_markets.first().cloned(),
            mode: None,
            source_url: Some(explicit_url.unwrap_or(&default_url).to_string()),
            table_selectors: default_table_selectors(),
            key_value_selectors: default_key_selectors(query),
            max_rows: Some(50),
            api: Some(ApiConfig {
                url: "https://push2.eastmoney.com/api/qt/ulist.np/get".to_string(),
                params: Some(build_default_api_params(query)),
                record_path: Some(vec!["data".to_string(), "diff".to_string()]),
                label_field: Some("f14".to_string()),
                price_field: Some("f2".to_string()),
                change_field: Some("f4".to_string()),
                change_pct_field: Some("f3".to_string()),
                ..Default::default()
            }),
        }
    }

    fn health_score(&self, source: &QuoteSourceConfig) -> i32 {
        let guard = self.health.read().expect("health lock");
        let Some(entry) = guard.get(&source.id) else {
            return 0;
        };
        if entry.healthy {
            2
        } else {
            1
        }
    }

    fn refresh_health(&self, force: bool) {
        if self.markets.is_empty() {
            return;
        }
        let mut needs_check = Vec::new();
        let now = Instant::now();
        {
            let guard = self.health.read().expect("health lock");
            for source in self.all_sources() {
                let should_check = guard
                    .get(&source.id)
                    .map(|status| {
                        status
                            .last_checked
                            .map(|t| now.duration_since(t) >= self.ttl)
                            .unwrap_or(true)
                    })
                    .unwrap_or(true);
                if force || should_check {
                    needs_check.push(source.id.clone());
                }
            }
        }
        for source_id in needs_check {
            if let Some(config) = self.find_source(&source_id) {
                let status = self.perform_health_check(config);
                self.health
                    .write()
                    .expect("health lock")
                    .insert(source_id.clone(), status);
            }
        }
    }

    fn all_sources(&self) -> impl Iterator<Item = &QuoteSourceConfig> {
        self.markets.iter().flat_map(|market| market.sources.iter())
    }

    fn find_source(&self, id: &str) -> Option<&QuoteSourceConfig> {
        self.all_sources().find(|cfg| cfg.id == id)
    }

    fn perform_health_check(&self, config: &QuoteSourceConfig) -> SourceHealth {
        if config.url_template.is_none() && config.api.is_none() {
            return SourceHealth {
                last_checked: Some(Instant::now()),
                healthy: false,
                message: Some("missing url".to_string()),
            };
        }

        SourceHealth {
            last_checked: Some(Instant::now()),
            healthy: true,
            message: None,
        }
    }

    fn mark_unhealthy(&self, source_id: &str, reason: &str) {
        if self.find_source(source_id).is_none() {
            return;
        }
        warn!(target = "quotes", id = %source_id, %reason, "marking quote source unhealthy from runtime" );
        self.health.write().expect("health lock").insert(
            source_id.to_string(),
            SourceHealth {
                last_checked: Some(Instant::now()),
                healthy: false,
                message: Some(reason.to_string()),
            },
        );
    }
}

fn source_priority(source: &QuoteSourceConfig, prefer_spot: bool) -> i32 {
    let is_spot = source.tags.iter().any(|tag| tag == "spot");
    let is_futures = source.tags.iter().any(|tag| tag == "futures");
    let is_eastmoney = source.id.contains("eastmoney");
    let is_sina = source.id.contains("sina");
    if prefer_spot {
        if is_spot && is_sina {
            4
        } else if is_spot && is_eastmoney {
            3
        } else if is_spot {
            2
        } else if is_futures && is_sina {
            1
        } else if is_futures && is_eastmoney {
            0
        } else if is_futures {
            -1
        } else {
            -2
        }
    } else {
        if is_futures && is_eastmoney {
            4
        } else if is_futures && is_sina {
            3
        } else if is_futures {
            2
        } else if is_spot && is_eastmoney {
            1
        } else if is_spot && is_sina {
            0
        } else if is_spot {
            -1
        } else {
            -2
        }
    }
}

fn dedup_payloads(mut items: Vec<QuoteFetchSourcePayload>) -> Vec<QuoteFetchSourcePayload> {
    let mut seen = HashSet::new();
    items.retain(|item| {
        if let Some(id) = &item.source_id {
            seen.insert(id.clone())
        } else if let Some(url) = &item.source_url {
            seen.insert(url.clone())
        } else {
            true
        }
    });
    items
}

impl ApiConfig {
    fn render(&self, query: QuoteQuery<'_>) -> ApiConfig {
        let mut cloned = self.clone();
        cloned.url = apply_template(&self.url, query);
        if let Some(params) = &self.params {
            let mut rendered = Map::new();
            for (key, value) in params {
                rendered.insert(key.clone(), render_value(value, query));
            }
            cloned.params = Some(rendered);
        }
        cloned
    }
}

fn render_value(value: &Value, query: QuoteQuery<'_>) -> Value {
    match value {
        Value::String(text) => Value::String(apply_template(text, query)),
        Value::Array(values) => {
            Value::Array(values.iter().map(|v| render_value(v, query)).collect())
        }
        Value::Object(map) => {
            let mut rendered = Map::new();
            for (key, val) in map {
                rendered.insert(key.clone(), render_value(val, query));
            }
            Value::Object(rendered)
        }
        _ => value.clone(),
    }
}

fn apply_template(template: &str, query: QuoteQuery<'_>) -> String {
    template
        .replace("{contract_upper}", &query.contract.to_ascii_uppercase())
        .replace("{contract_lower}", &query.contract.to_ascii_lowercase())
        .replace("{slug}", query.slug)
        .replace("{metal_label}", query.metal_label)
}

fn explicit_matches_query(url: &str, query: QuoteQuery<'_>) -> bool {
    let lower = url.to_ascii_lowercase();
    let contract = query.contract.to_ascii_lowercase();
    let slug = query.slug.to_ascii_lowercase();

    if lower.contains(&contract) || lower.contains(&slug) {
        return true;
    }

    if let Some(explicit_contract) = contract_from_url(&lower) {
        return explicit_contract == contract;
    }

    if let Some(explicit_slug) = metal_slug_from_url(&lower) {
        return explicit_slug == slug;
    }

    false
}

fn contract_from_url(url: &str) -> Option<String> {
    segment_after(url, "/qh/")
        .map(|segment| {
            segment
                .split(|ch| matches!(ch, '.' | '/' | '?'))
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .filter(|value| !value.is_empty())
}

fn metal_slug_from_url(url: &str) -> Option<String> {
    segment_after(url, "/metal/")
        .map(|segment| {
            segment
                .split(|ch| matches!(ch, '.' | '/' | '?'))
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .filter(|value| !value.is_empty())
}

fn segment_after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let idx = text.find(marker)?;
    let start = idx + marker.len();
    text.get(start..)
}

fn default_table_selectors() -> Vec<String> {
    vec![
        "table.hq_table".to_string(),
        "table.dataTable".to_string(),
        ".quote-table table".to_string(),
    ]
}

fn default_key_selectors(query: QuoteQuery<'_>) -> Vec<SelectorConfig> {
    vec![
        SelectorConfig {
            label: Some(format!("{}主力", query.metal_label)),
            selector: Some(".hq_table tr:first-child".to_string()),
            attribute: None,
        },
        SelectorConfig {
            label: Some(format!("{}最新价", query.metal_label)),
            selector: Some(".price,.quote-price".to_string()),
            attribute: None,
        },
    ]
}

fn build_default_api_params(query: QuoteQuery<'_>) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("pn".to_string(), Value::String("1".to_string()));
    params.insert("pz".to_string(), Value::String("5".to_string()));
    params.insert("fl".to_string(), Value::String("1".to_string()));
    params.insert(
        "secids".to_string(),
        Value::String(format!("115.{}", query.contract.to_ascii_uppercase())),
    );
    params.insert(
        "fields".to_string(),
        Value::String("f1,f2,f3,f4,f12,f13,f14".to_string()),
    );
    params
}
