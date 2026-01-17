use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::error::Error;
use std::fmt;

/// Normalize schema identifiers (remove extension, lowercase).
pub fn canonical_schema_id(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(".json")
        .to_ascii_lowercase()
        .to_string()
}

/// Validate structured output values according to known schema contracts.
pub fn validate_structured_output(schema: &str, value: &Value) -> Result<()> {
    let canonical = canonical_schema_id(schema);
    if let Some(actual) = value.get("schema").and_then(Value::as_str) {
        let normalized_actual = canonical_schema_id(actual);
        if normalized_actual != canonical {
            bail!(
                "structured output schema mismatch: expected '{}' but value reports '{}'",
                canonical,
                normalized_actual
            );
        }
    }

    match canonical.as_str() {
        "market_info_v1" => validate_market_info(value),
        "metal_price_v1" => validate_metal_price(value),
        "news_brief_v1" => validate_news_brief(value),
        "github_repos_v1" => validate_github_repos(value),
        "twitter_feed_v1" => validate_twitter_feed(value),
        "facebook_feed_v1" => validate_facebook_feed(value),
        "hackernews_feed_v1" => validate_hackernews_feed(value),
        "weather_report_v1" => validate_weather_report(value),
        _ => Ok(()),
    }
}

fn validate_metal_price(value: &Value) -> Result<()> {
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("metal_price_v1 requires 'items' array"))?;
    if items.is_empty() {
        bail!("metal_price_v1 requires at least one entry");
    }
    for (idx, item) in items.iter().enumerate() {
        let metal = item
            .get("metal")
            .and_then(Value::as_str)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("items[{}] missing 'metal'", idx))?;
        let price = item
            .get("price")
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow!("items[{}] missing numeric 'price'", idx))?;
        if !price.is_finite() || price <= 0.0 {
            bail!("items[{}] has invalid price", idx);
        }
        if let Some(currency) = item.get("currency").and_then(Value::as_str) {
            if currency.trim().is_empty() {
                bail!("items[{}] contains empty 'currency'", idx);
            }
        }
        if let Some(unit) = item.get("unit").and_then(Value::as_str) {
            if unit.trim().is_empty() {
                bail!("items[{}] contains empty 'unit'", idx);
            }
        }
        if let Some(change) = item.get("change") {
            change
                .as_f64()
                .ok_or_else(|| anyhow!("items[{}] contains non-numeric 'change'", idx))?;
        }
        if let Some(change_pct) = item.get("change_pct") {
            change_pct
                .as_f64()
                .ok_or_else(|| anyhow!("items[{}] contains non-numeric 'change_pct'", idx))?;
        }
        if let Some(as_of) = item.get("as_of").and_then(Value::as_str) {
            if as_of.trim().is_empty() {
                bail!("items[{}] contains empty 'as_of'", idx);
            }
        }
        if metal.is_empty() {
            bail!("items[{}] contains empty 'metal'", idx);
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum MetalPriceValidationFailure {
    MissingMetal(String),
    MissingMarket(Vec<String>),
    StaleQuotes { max_age_hours: f64 },
}

impl fmt::Display for MetalPriceValidationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetalPriceValidationFailure::MissingMetal(keyword) => {
                write!(f, "未找到匹配 {} 的金属行情", keyword)
            }
            MetalPriceValidationFailure::MissingMarket(markets) => {
                write!(f, "抓取结果未覆盖期望市场: {}", markets.join("/"))
            }
            MetalPriceValidationFailure::StaleQuotes { max_age_hours } => {
                write!(f, "行情时间超过 {} 小时", max_age_hours)
            }
        }
    }
}

impl Error for MetalPriceValidationFailure {}

pub struct MetalPriceValidationContext<'a> {
    pub metal_keyword: &'a str,
    pub allowed_markets: &'a [String],
    pub max_age_hours: f64,
}

#[derive(Debug, Clone)]
pub struct MetalPriceValidationReport {
    pub total_items: usize,
    pub matched_metal: usize,
    pub matched_market: usize,
    pub fresh_entries: usize,
    pub newest_as_of: Option<String>,
}

pub fn validate_metal_price_with_context(
    value: &Value,
    context: &MetalPriceValidationContext,
) -> Result<MetalPriceValidationReport> {
    validate_metal_price(value)?;
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("metal_price_v1 requires 'items' array"))?;
    let keyword = context.metal_keyword.trim();
    let allowed_markets = context.allowed_markets;
    let threshold_secs = (context.max_age_hours.max(1.0) * 3600.0) as i64;
    let freshness_threshold = Utc::now() - Duration::seconds(threshold_secs);

    let mut matched_metal = 0usize;
    let mut matched_market = 0usize;
    let mut fresh_entries = 0usize;
    let mut newest: Option<DateTime<Utc>> = None;
    let mut newest_str: Option<String> = None;

    for item in items {
        let metal = item.get("metal").and_then(Value::as_str).unwrap_or("");
        if !keyword.is_empty() && contains_insensitive(metal, keyword) {
            matched_metal += 1;
        }

        if !allowed_markets.is_empty() {
            let market = item.get("market").and_then(Value::as_str).unwrap_or("");
            if allowed_markets
                .iter()
                .any(|expected| contains_insensitive(market, expected))
            {
                matched_market += 1;
            }
        }

        if let Some(as_of) = item.get("as_of").and_then(Value::as_str) {
            if let Some(parsed) = parse_timestamp(as_of) {
                if newest.map(|current| parsed > current).unwrap_or(true) {
                    newest = Some(parsed);
                    newest_str = Some(as_of.to_string());
                }
                if parsed >= freshness_threshold {
                    fresh_entries += 1;
                }
            }
        }
    }

    if !keyword.is_empty() && matched_metal == 0 {
        return Err(MetalPriceValidationFailure::MissingMetal(keyword.to_string()).into());
    }
    if !allowed_markets.is_empty() && matched_market == 0 {
        return Err(MetalPriceValidationFailure::MissingMarket(allowed_markets.to_vec()).into());
    }
    if fresh_entries == 0 {
        return Err(MetalPriceValidationFailure::StaleQuotes {
            max_age_hours: context.max_age_hours,
        }
        .into());
    }

    Ok(MetalPriceValidationReport {
        total_items: items.len(),
        matched_metal,
        matched_market,
        fresh_entries,
        newest_as_of: newest_str,
    })
}

/// Generate a short human readable summary for structured payloads when possible.
pub fn summarize_structured_output(schema: &str, value: &Value) -> Option<String> {
    match canonical_schema_id(schema).as_str() {
        "weather_report_v1" => summarize_weather_report(value),
        _ => None,
    }
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}

fn contains_insensitive(haystack: &str, needle: &str) -> bool {
    if haystack.is_empty() || needle.trim().is_empty() {
        return false;
    }
    let lower = haystack.to_ascii_lowercase();
    lower.contains(&needle.to_ascii_lowercase()) || haystack.contains(needle)
}

fn validate_market_info(value: &Value) -> Result<()> {
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("market_info_v1 requires 'items' array"))?;
    if items.is_empty() {
        bail!("market_info_v1 requires at least one index entry");
    }
    for (idx, item) in items.iter().enumerate() {
        let name = item
            .get("index_name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow!("items[{}] missing 'index_name'", idx))?;
        let value_field = item
            .get("value")
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow!("items[{}] missing numeric 'value'", idx))?;
        let change = item
            .get("change")
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow!("items[{}] missing numeric 'change'", idx))?;
        let change_pct = item
            .get("change_pct")
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow!("items[{}] missing numeric 'change_pct'", idx))?;
        if !name.is_empty()
            && value_field.is_finite()
            && change.is_finite()
            && change_pct.is_finite()
        {
            continue;
        } else {
            bail!("items[{}] contains invalid data", idx);
        }
    }
    Ok(())
}

fn validate_news_brief(value: &Value) -> Result<()> {
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("news_brief_v1 requires 'items' array"))?;
    if items.is_empty() {
        bail!("news_brief_v1 requires at least one entry");
    }
    for (idx, item) in items.iter().enumerate() {
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow!("items[{}] missing 'title'", idx))?;
        let summary = item
            .get("summary")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow!("items[{}] missing 'summary'", idx))?;
        let url = item
            .get("url")
            .and_then(Value::as_str)
            .filter(|s| s.starts_with("http"))
            .ok_or_else(|| anyhow!("items[{}] missing valid 'url'", idx))?;
        let _ = (title, summary, url);
        if let Some(source) = item.get("source").and_then(Value::as_str) {
            if source.trim().is_empty() {
                bail!("items[{}] contains empty 'source'", idx);
            }
        }
    }
    Ok(())
}

fn validate_weather_report(value: &Value) -> Result<()> {
    let city = value
        .get("city")
        .and_then(Value::as_str)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("weather_report_v1 requires 'city'"))?;
    if city.is_empty() {
        bail!("weather_report_v1 requires 'city'");
    }

    let condition = value
        .get("condition")
        .and_then(Value::as_str)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("weather_report_v1 requires 'condition'"))?;
    if condition.is_empty() {
        bail!("weather_report_v1 requires 'condition'");
    }

    let high = value
        .get("temperature_high_c")
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("weather_report_v1 requires 'temperature_high_c'"))?;
    let low = value
        .get("temperature_low_c")
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("weather_report_v1 requires 'temperature_low_c'"))?;

    if high < low {
        bail!("temperature_high_c must be >= temperature_low_c");
    }

    Ok(())
}

fn summarize_weather_report(value: &Value) -> Option<String> {
    let city = value
        .get("city")
        .and_then(Value::as_str)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())?;
    let condition = value
        .get("condition")
        .and_then(Value::as_str)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())?;
    let high = value.get("temperature_high_c").and_then(Value::as_f64)?;
    let low = value.get("temperature_low_c").and_then(Value::as_f64)?;

    Some(format!(
        "{}: {}, {}°C-{}°C",
        city,
        condition,
        format_temp(low),
        format_temp(high)
    ))
}

fn format_temp(value: f64) -> String {
    if (value.fract()).abs() < f64::EPSILON {
        format!("{:.0}", value)
    } else {
        format!("{:.1}", value)
    }
}

fn validate_github_repos(value: &Value) -> Result<()> {
    let _source = value
        .get("source_username")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow!("github_repos_v1 requires 'source_username'"))?;
    let _fetched = value
        .get("fetched_at")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow!("github_repos_v1 requires 'fetched_at' timestamp"))?;

    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("github_repos_v1 requires 'items' array"))?;

    for (idx, item) in items.iter().enumerate() {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow!("items[{}] missing 'name'", idx))?;
        if item
            .get("html_url")
            .and_then(Value::as_str)
            .filter(|s| s.starts_with("http"))
            .is_none()
        {
            bail!("items[{}] missing 'html_url'", idx);
        }
        for key in ["stars", "forks", "watchers", "open_issues"] {
            if item.get(key).and_then(Value::as_u64).is_none() {
                bail!("items[{}] missing numeric '{}'", idx, key);
            }
        }
        if item.get("archived").and_then(Value::as_bool).is_none() {
            bail!("items[{}] missing 'archived' flag", idx);
        }
        if item.get("disabled").and_then(Value::as_bool).is_none() {
            bail!("items[{}] missing 'disabled' flag", idx);
        }
        if item.get("is_fork").and_then(Value::as_bool).is_none() {
            bail!("items[{}] missing 'is_fork' flag", idx);
        }
        if item
            .get("visibility")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .is_none()
        {
            bail!("items[{}] missing 'visibility'", idx);
        }
        if item
            .get("default_branch")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .is_none()
        {
            bail!("items[{}] missing 'default_branch'", idx);
        }
        let _ = name;
    }
    Ok(())
}

fn validate_twitter_feed(value: &Value) -> Result<()> {
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("twitter_feed_v1 requires 'items' array"))?;
    if items.is_empty() {
        bail!("twitter_feed_v1 requires at least one tweet");
    }
    if let Some(handle) = value.get("account_handle").and_then(Value::as_str) {
        if !handle.starts_with('@') {
            bail!("account_handle must start with '@'");
        }
    }
    for (idx, item) in items.iter().enumerate() {
        let url = item
            .get("url")
            .and_then(Value::as_str)
            .filter(|s| s.starts_with("http"))
            .ok_or_else(|| anyhow!("items[{}] missing valid 'url'", idx))?;
        let author_handle = item
            .get("author_handle")
            .and_then(Value::as_str)
            .filter(|s| s.starts_with('@'))
            .ok_or_else(|| anyhow!("items[{}] missing '@author_handle'", idx))?;
        let content = item
            .get("content")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow!("items[{}] missing 'content'", idx))?;
        let _ = (url, author_handle, content);
    }
    Ok(())
}

fn validate_facebook_feed(value: &Value) -> Result<()> {
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("facebook_feed_v1 requires 'items' array"))?;
    if items.is_empty() {
        bail!("facebook_feed_v1 requires at least one post");
    }
    for (idx, item) in items.iter().enumerate() {
        item.get("url")
            .and_then(Value::as_str)
            .filter(|s| s.starts_with("http"))
            .ok_or_else(|| anyhow!("items[{}] missing valid 'url'", idx))?;
        let content = item
            .get("content")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow!("items[{}] missing 'content'", idx))?;
        if let Some(author) = item.get("author_name").and_then(Value::as_str) {
            if author.trim().is_empty() {
                bail!("items[{}] contains empty 'author_name'", idx);
            }
        }
        let _ = content;
    }
    Ok(())
}

fn validate_hackernews_feed(value: &Value) -> Result<()> {
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("hackernews_feed_v1 requires 'items' array"))?;
    if items.is_empty() {
        bail!("hackernews_feed_v1 requires at least one story");
    }
    for (idx, item) in items.iter().enumerate() {
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow!("items[{}] missing 'title'", idx))?;
        let url = item
            .get("url")
            .and_then(Value::as_str)
            .filter(|s| s.starts_with("http"))
            .ok_or_else(|| anyhow!("items[{}] missing valid 'url'", idx))?;
        if let Some(rank) = item.get("rank") {
            let value = rank
                .as_u64()
                .ok_or_else(|| anyhow!("items[{}] rank must be positive integer", idx))?;
            if value == 0 {
                bail!("items[{}] rank must be >= 1", idx);
            }
        }
        if let Some(points) = item.get("points") {
            points
                .as_u64()
                .ok_or_else(|| anyhow!("items[{}] points must be non-negative", idx))?;
        }
        if let Some(comments) = item.get("comment_count") {
            comments
                .as_u64()
                .ok_or_else(|| anyhow!("items[{}] comment_count must be non-negative", idx))?;
        }
        if let Some(author) = item.get("author").and_then(Value::as_str) {
            if author.trim().is_empty() {
                bail!("items[{}] contains empty 'author'", idx);
            }
        }
        if let Some(discussion) = item.get("discussion_url").and_then(Value::as_str) {
            if !discussion.starts_with("http") {
                bail!("items[{}] discussion_url must be absolute", idx);
            }
        }
        let _ = (title, url);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_market_info() {
        let value = json!({
            "schema": "market_info_v1",
            "items": [
                {
                    "index_name": "上证指数",
                    "value": 3011.4,
                    "change": -12.3,
                    "change_pct": -0.41
                }
            ]
        });
        validate_structured_output("market_info_v1", &value).expect("valid schema");
    }

    #[test]
    fn detects_missing_items() {
        let value = json!({
            "schema": "market_info_v1",
            "items": []
        });
        assert!(validate_structured_output("market_info_v1", &value).is_err());
    }

    #[test]
    fn validates_metal_price() {
        let value = json!({
            "schema": "metal_price_v1",
            "items": [
                {
                    "metal": "沪铜主连",
                    "price": 60590.0,
                    "currency": "CNY",
                    "unit": "元/吨",
                    "change": -120.0,
                    "change_pct": -0.2,
                    "as_of": "2026-01-03T02:10:00Z"
                }
            ]
        });
        validate_structured_output("metal_price_v1", &value).expect("valid schema");
    }

    #[test]
    fn rejects_invalid_metal_price() {
        let value = json!({
            "schema": "metal_price_v1",
            "items": [
                {
                    "metal": "",
                    "price": -1.0
                }
            ]
        });
        assert!(validate_structured_output("metal_price_v1", &value).is_err());
    }

    #[test]
    fn validates_metal_price_context_success() {
        let now = Utc::now().to_rfc3339();
        let value = json!({
            "schema": "metal_price_v1",
            "items": [
                {
                    "metal": "沪铜主连",
                    "market": "上海期货交易所",
                    "price": 62000.0,
                    "as_of": now
                }
            ]
        });
        let ctx = MetalPriceValidationContext {
            metal_keyword: "铜",
            allowed_markets: &["上海期货交易所".to_string()],
            max_age_hours: 24.0,
        };
        let report = validate_metal_price_with_context(&value, &ctx).expect("valid context");
        assert_eq!(report.fresh_entries, 1);
        assert_eq!(report.matched_metal, 1);
        assert_eq!(report.total_items, 1);
    }

    #[test]
    fn rejects_stale_metal_price_context() {
        let stale = (Utc::now() - Duration::hours(30)).to_rfc3339();
        let value = json!({
            "schema": "metal_price_v1",
            "items": [
                {
                    "metal": "沪铜主连",
                    "market": "上海期货交易所",
                    "price": 60000.0,
                    "as_of": stale
                }
            ]
        });
        let ctx = MetalPriceValidationContext {
            metal_keyword: "铜",
            allowed_markets: &[],
            max_age_hours: 24.0,
        };
        assert!(validate_metal_price_with_context(&value, &ctx).is_err());
    }

    #[test]
    fn validates_news_brief() {
        let value = json!({
            "schema": "news_brief_v1",
            "items": [
                {
                    "title": "Headline",
                    "summary": "Quick summary",
                    "url": "https://example.com/article",
                    "source": "Example"
                }
            ]
        });
        validate_structured_output("news_brief_v1", &value).expect("valid schema");
    }

    #[test]
    fn news_brief_requires_url() {
        let value = json!({
            "schema": "news_brief_v1",
            "items": [
                {
                    "title": "Headline",
                    "summary": "Summary"
                }
            ]
        });
        assert!(validate_structured_output("news_brief_v1", &value).is_err());
    }

    #[test]
    fn validates_github_repos() {
        let value = json!({
            "schema": "github_repos_v1",
            "source_username": "octocat",
            "fetched_at": "2025-11-27T00:00:00Z",
            "items": [
                {
                    "name": "hello-world",
                    "html_url": "https://github.com/octocat/hello-world",
                    "stars": 42,
                    "forks": 5,
                    "watchers": 42,
                    "open_issues": 0,
                    "visibility": "public",
                    "default_branch": "main",
                    "archived": false,
                    "disabled": false,
                    "is_fork": false
                }
            ]
        });
        validate_structured_output("github_repos_v1", &value).expect("valid github schema");
    }

    #[test]
    fn github_repos_requires_numeric_fields() {
        let value = json!({
            "schema": "github_repos_v1",
            "source_username": "octocat",
            "fetched_at": "2025-11-27T00:00:00Z",
            "items": [
                {
                    "name": "hello-world",
                    "html_url": "https://github.com/octocat/hello-world",
                    "stars": "invalid"
                }
            ]
        });
        assert!(validate_structured_output("github_repos_v1", &value).is_err());
    }

    #[test]
    fn validates_twitter_feed() {
        let value = json!({
            "schema": "twitter_feed_v1",
            "items": [
                {
                    "url": "https://twitter.com/soulbrowser/status/1",
                    "author_handle": "@soulbrowser",
                    "content": "First tweet"
                }
            ]
        });
        validate_structured_output("twitter_feed_v1", &value).expect("valid twitter schema");
    }

    #[test]
    fn twitter_feed_requires_handles() {
        let value = json!({
            "schema": "twitter_feed_v1",
            "items": [
                {
                    "url": "https://twitter.com/soulbrowser/status/1",
                    "content": "First tweet"
                }
            ]
        });
        assert!(validate_structured_output("twitter_feed_v1", &value).is_err());
    }

    #[test]
    fn validates_facebook_feed() {
        let value = json!({
            "schema": "facebook_feed_v1",
            "items": [
                {
                    "url": "https://www.facebook.com/soulbrowser/posts/1",
                    "content": "Hello Facebook"
                }
            ]
        });
        validate_structured_output("facebook_feed_v1", &value).expect("valid facebook schema");
    }

    #[test]
    fn facebook_feed_requires_content() {
        let value = json!({
            "schema": "facebook_feed_v1",
            "items": [
                {
                    "url": "https://www.facebook.com/soulbrowser/posts/1"
                }
            ]
        });
        assert!(validate_structured_output("facebook_feed_v1", &value).is_err());
    }

    #[test]
    fn validates_hackernews_feed() {
        let value = json!({
            "schema": "hackernews_feed_v1",
            "items": [
                {
                    "rank": 1,
                    "title": "Example",
                    "url": "https://example.com/story",
                    "discussion_url": "https://news.ycombinator.com/item?id=1",
                    "points": 123,
                    "author": "alice",
                    "comment_count": 45
                }
            ]
        });
        validate_structured_output("hackernews_feed_v1", &value).expect("valid hackernews schema");
    }

    #[test]
    fn hackernews_feed_requires_title() {
        let value = json!({
            "schema": "hackernews_feed_v1",
            "items": [
                {
                    "url": "https://example.com/story"
                }
            ]
        });
        assert!(validate_structured_output("hackernews_feed_v1", &value).is_err());
    }

    #[test]
    fn validates_weather_report_schema() {
        let value = json!({
            "schema": "weather_report_v1",
            "city": "北京",
            "condition": "晴",
            "temperature_high_c": 6.0,
            "temperature_low_c": 0.0
        });
        validate_structured_output("weather_report_v1", &value).expect("valid weather schema");
    }

    #[test]
    fn summarizes_weather_payload() {
        let value = json!({
            "schema": "weather_report_v1",
            "city": "Beijing",
            "condition": "Sunny",
            "temperature_high_c": 6.0,
            "temperature_low_c": 0.0
        });
        let summary = summarize_structured_output("weather_report_v1", &value).expect("summary");
        assert_eq!(summary, "Beijing: Sunny, 0°C-6°C");
    }
}
