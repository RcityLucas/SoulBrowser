use std::collections::HashSet;

use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::parsers::helpers::normalize_whitespace;
use crate::parsers::helpers::{extract_observation_metadata, ObservationMetadata};

const SCHEMA_ID: &str = "metal_price_v1";
static NUMBER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[-+]?\d+(?:\.\d+)?").unwrap());
static TEXT_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?P<label>[^\s:：]{0,8}(?:铜|银|金|铝|镍|锌|铅|锡)[^\s:：]{0,6}).{0,12}?(?P<value>[-+]?\d+(?:\.\d+)?)")
        .unwrap()
});

static METAL_PATTERNS: &[&[&str]] = &[
    &["白银", "银", "ag", "silver"],
    &["黄金", "金", "au", "gold"],
    &["铜", "cu", "copper"],
    &["铝", "al", "aluminum"],
    &["镍", "ni", "nickel"],
    &["锌", "zn", "zinc"],
    &["铅", "pb", "lead"],
    &["锡", "sn", "tin"],
];

#[derive(Debug, Clone, Serialize)]
pub struct MetalPriceItem {
    pub metal: String,
    pub price: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_text: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MetalPriceOutput {
    pub schema: &'static str,
    pub source_url: Option<String>,
    pub captured_at: Option<String>,
    pub items: Vec<MetalPriceItem>,
}

pub fn parse_metal_price(observation: &Value) -> Result<Value> {
    let metadata = extract_observation_metadata(observation);
    let mut items = Vec::new();

    items.extend(parse_from_tables(observation, &metadata));
    items.extend(parse_from_key_values(observation, &metadata));
    items.extend(parse_from_text_sample(observation, &metadata));

    let mut dedup = Vec::new();
    let mut seen = HashSet::new();
    for item in items
        .into_iter()
        .filter(|item| item.price.is_finite() && item.price > 0.0)
    {
        let key = format!(
            "{}|{}|{}",
            item.metal.to_lowercase(),
            item.contract.as_deref().unwrap_or("").to_ascii_lowercase(),
            item.market.as_deref().unwrap_or("").to_ascii_lowercase()
        );
        if seen.insert(key) {
            dedup.push(item);
        }
    }

    if dedup.is_empty() {
        bail!("unable to extract metal price entries from observation");
    }

    let output = MetalPriceOutput {
        schema: SCHEMA_ID,
        source_url: metadata.source_url,
        captured_at: metadata.captured_at,
        items: dedup,
    };

    serde_json::to_value(output).context("serialize metal price output")
}

fn parse_from_tables(observation: &Value, metadata: &ObservationMetadata) -> Vec<MetalPriceItem> {
    let mut items = Vec::new();
    let tables = match observation.get("tables").and_then(Value::as_array) {
        Some(tables) if !tables.is_empty() => tables,
        _ => return items,
    };
    let metadata_title = metadata.primary_title();

    for table in tables {
        let headers = table
            .get("headers")
            .and_then(Value::as_array)
            .map(|array| extract_text_array(array))
            .unwrap_or_default();
        let rows = table.get("rows").and_then(Value::as_array);
        if rows.is_none() {
            continue;
        }
        let mut price_idx = None;
        let mut change_idx = None;
        let mut change_pct_idx = None;
        let mut contract_idx = None;
        let mut market_idx = None;

        for (idx, header) in headers.iter().enumerate() {
            let lower = header.to_ascii_lowercase();
            if price_idx.is_none()
                && (lower.contains("价") || lower.contains("price") || lower.contains("latest"))
            {
                price_idx = Some(idx);
            }
            if change_idx.is_none() && (lower.contains("涨跌") || lower.contains("change")) {
                change_idx = Some(idx);
            }
            if change_pct_idx.is_none()
                && (lower.contains("涨跌幅") || lower.contains("%") || lower.contains("pct"))
            {
                change_pct_idx = Some(idx);
            }
            if contract_idx.is_none() && lower.contains("合约") {
                contract_idx = Some(idx);
            }
            if market_idx.is_none()
                && (lower.contains("市场") || lower.contains("交易") || lower.contains("exchange"))
            {
                market_idx = Some(idx);
            }
        }

        for row in rows.unwrap() {
            let cells = extract_text_array_from_row(row);
            if cells.is_empty() {
                continue;
            }
            let name_cell = cells.get(0).cloned().unwrap_or_default();
            if !is_metal_candidate(&name_cell) {
                continue;
            }
            let price_value = price_idx
                .and_then(|idx| cells.get(idx).cloned())
                .or_else(|| cells.get(1).cloned())
                .and_then(|text| parse_number(&text));
            if price_value.is_none() {
                continue;
            }

            let change_value = change_idx
                .and_then(|idx| cells.get(idx))
                .and_then(|text| parse_number(text));
            let change_pct_value = change_pct_idx
                .and_then(|idx| cells.get(idx))
                .and_then(|text| parse_number(text));
            let contract = contract_idx.and_then(|idx| cells.get(idx).cloned());
            let market = market_idx.and_then(|idx| cells.get(idx).cloned());
            let hint_context = market
                .as_deref()
                .or(metadata_title.as_deref())
                .unwrap_or("");
            let (market_label, currency, unit) = infer_market_details(&name_cell, hint_context);

            items.push(MetalPriceItem {
                metal: extract_metal_label(&name_cell),
                price: price_value.unwrap(),
                contract,
                market: market_label,
                currency,
                unit,
                change: change_value,
                change_pct: change_pct_value,
                as_of: metadata.captured_at.clone(),
                source_text: Some(name_cell.clone()),
            });
        }
    }

    items
}

fn parse_from_key_values(
    observation: &Value,
    metadata: &ObservationMetadata,
) -> Vec<MetalPriceItem> {
    let mut items = Vec::new();
    let entries = observation
        .get("key_values")
        .and_then(Value::as_array)
        .filter(|arr| !arr.is_empty());
    if entries.is_none() {
        return items;
    }

    for entry in entries.unwrap() {
        let label = entry
            .get("label")
            .and_then(Value::as_str)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        if !is_metal_candidate(label) {
            continue;
        }
        let value_text = entry
            .get("value")
            .and_then(Value::as_str)
            .map(|s| s.trim())
            .unwrap_or("");
        if let Some(price) = parse_number(value_text) {
            let (market, currency, unit) = infer_market_details(label, value_text);
            items.push(MetalPriceItem {
                metal: extract_metal_label(label),
                price,
                contract: infer_contract(label),
                market,
                currency,
                unit,
                change: None,
                change_pct: None,
                as_of: metadata.captured_at.clone(),
                source_text: Some(value_text.to_string()),
            });
        }
    }

    items
}

fn parse_from_text_sample(
    observation: &Value,
    metadata: &ObservationMetadata,
) -> Vec<MetalPriceItem> {
    let mut items = Vec::new();
    let text = observation
        .get("text_sample")
        .and_then(Value::as_str)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    if text.is_none() {
        return items;
    }

    let normalized = normalize_whitespace(text.unwrap());
    for caps in TEXT_PATTERN.captures_iter(&normalized) {
        let label = caps.name("label").map(|m| m.as_str()).unwrap_or("");
        let price = caps
            .name("value")
            .and_then(|m| m.as_str().parse::<f64>().ok())
            .filter(|value| *value > 0.0);
        if !is_metal_candidate(label) || price.is_none() {
            continue;
        }
        // Filter out percentages or suspiciously small values (< 10)
        if price.unwrap() < 10.0 && !label.contains('%') {
            continue;
        }
        let (market, currency, unit) = infer_market_details(label, &normalized);
        items.push(MetalPriceItem {
            metal: extract_metal_label(label),
            price: price.unwrap(),
            contract: infer_contract(label),
            market,
            currency,
            unit,
            change: None,
            change_pct: None,
            as_of: metadata.captured_at.clone(),
            source_text: Some(label.to_string()),
        });
    }

    items
}

fn extract_text_array(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .map(|value| match value {
            Value::String(text) => text.clone(),
            Value::Object(obj) => obj
                .get("text")
                .or_else(|| obj.get("value"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            other => other.to_string(),
        })
        .collect()
}

fn extract_text_array_from_row(row: &Value) -> Vec<String> {
    if let Some(array) = row.as_array() {
        return extract_text_array(array);
    }
    if let Some(obj) = row.as_object() {
        if let Some(cells) = obj.get("cells").and_then(Value::as_array) {
            return extract_text_array(cells);
        }
    }
    Vec::new()
}

fn is_metal_candidate(text: &str) -> bool {
    let normalized = text.trim();
    if normalized.is_empty() {
        return false;
    }
    let lower = normalized.to_ascii_lowercase();
    METAL_PATTERNS.iter().any(|patterns| {
        patterns
            .iter()
            .any(|pattern| contains_pattern(normalized, &lower, pattern))
    })
}

fn extract_metal_label(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "金属".to_string()
    } else {
        trimmed
            .trim_matches(|ch: char| ch.is_whitespace() || ch == ':' || ch == '：')
            .to_string()
    }
}

fn contains_pattern(original: &str, lower: &str, pattern: &str) -> bool {
    if pattern.chars().any(|ch| !ch.is_ascii()) {
        original.contains(pattern)
    } else {
        lower.contains(&pattern.to_ascii_lowercase())
    }
}

fn parse_number(text: &str) -> Option<f64> {
    let cleaned = text.trim();
    if cleaned.is_empty() {
        return None;
    }
    if cleaned.contains('%') {
        return None;
    }
    NUMBER_RE
        .find(cleaned)
        .and_then(|m| m.as_str().parse::<f64>().ok())
}

fn infer_contract(label: &str) -> Option<String> {
    let stripped = label
        .split_whitespace()
        .next()
        .unwrap_or(label)
        .trim_matches(|ch: char| ch == ':' || ch == '：');
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

fn infer_market_details(
    label: &str,
    hint: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let lower_label = label.to_ascii_lowercase();
    let lower_hint = hint.to_ascii_lowercase();
    if lower_label.contains('沪') || lower_label.contains("shfe") || lower_hint.contains("上海")
    {
        (
            Some("上海期货交易所".to_string()),
            Some("CNY".to_string()),
            Some("元/吨".to_string()),
        )
    } else if lower_label.contains('伦') || lower_hint.contains("lme") {
        (
            Some("伦敦金属交易所".to_string()),
            Some("USD".to_string()),
            Some("美元/吨".to_string()),
        )
    } else {
        (None, None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_from_table_rows() {
        let observation = json!({
            "url": "https://example.com/cu",
            "fetched_at": "2026-01-03T02:10:00Z",
            "tables": [
                {
                    "headers": ["品种", "最新价", "涨跌", "涨跌幅"],
                    "rows": [
                        ["沪铜主连", "60590", "-120", "-0.20%"],
                        ["伦铜", "8450", "+15", "+0.18%"]
                    ]
                }
            ]
        });

        let result = parse_metal_price(&observation).expect("parsed output");
        assert_eq!(result["schema"], SCHEMA_ID);
        assert_eq!(result["items"].as_array().unwrap().len(), 2);
        assert_eq!(result["items"][0]["metal"], "沪铜主连");
        assert_eq!(result["items"][0]["price"], 60590.0);
    }

    #[test]
    fn parses_from_key_values() {
        let observation = json!({
            "fetched_at": "2026-01-03T02:10:00Z",
            "key_values": [
                {"label": "沪铜主连", "value": "60590 元/吨"},
                {"label": "伦铜", "value": "8450 美元/吨"}
            ]
        });

        let result = parse_metal_price(&observation).expect("parsed output");
        assert_eq!(result["items"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn errors_when_no_entries() {
        let observation = json!({"text_sample": "随机文本"});
        assert!(parse_metal_price(&observation).is_err());
    }

    #[test]
    fn detects_silver_and_gold_candidates() {
        assert!(is_metal_candidate("白银主连"));
        assert!(is_metal_candidate("AG0"));
        assert!(is_metal_candidate("伦敦SILVER"));
        assert!(is_metal_candidate("黄金主连"));
        assert!(!is_metal_candidate("原油期货"));
        assert_eq!(extract_metal_label("白银主连"), "白银主连");
        assert_eq!(extract_metal_label("黄金主连"), "黄金主连");
    }

    #[test]
    fn parses_silver_rows_from_table() {
        let observation = json!({
            "url": "https://quote.eastmoney.com/qh/AG0.html",
            "fetched_at": "2026-01-03T02:10:00Z",
            "tables": [
                {
                    "headers": ["品种", "最新价", "涨跌幅"],
                    "rows": [
                        ["白银主连", "24.50", "0.32%"],
                        ["黄金主连", "450.20", "-0.11%"]
                    ]
                }
            ]
        });

        let result = parse_metal_price(&observation).expect("parsed output");
        let items = result["items"].as_array().expect("items array");
        assert_eq!(items.len(), 2);
        let silver = items
            .iter()
            .find(|item| item.get("metal").and_then(Value::as_str) == Some("白银主连"))
            .expect("silver item");
        assert_eq!(silver.get("price").and_then(Value::as_f64), Some(24.5));
    }
}
