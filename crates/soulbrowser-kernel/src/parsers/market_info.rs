use anyhow::{anyhow, bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::parsers::extract_observation_metadata;

const SCHEMA_ID: &str = "market_info_v1";
static NUMBER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[-+]?\d+(?:\.\d+)?").unwrap());

const INDEX_LABELS: &[&str] = &[
    "上证指数",
    "深证成指",
    "创业板指",
    "沪深300",
    "上证50",
    "中小100",
    "中证500",
    "上证180",
    "上证380",
    "A股指数",
];

#[derive(Debug, Serialize)]
pub struct MarketInfoItem {
    pub index_name: String,
    pub value: f64,
    pub change: f64,
    pub change_pct: f64,
}

#[derive(Debug, Serialize)]
pub struct MarketInfoOutput {
    pub schema: &'static str,
    pub source_url: Option<String>,
    pub captured_at: Option<String>,
    pub items: Vec<MarketInfoItem>,
}

pub fn parse_market_info(observation: &Value) -> Result<Value> {
    let metadata = extract_observation_metadata(observation);
    let text_sample = observation
        .get("text_sample")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow!("observation missing text sample for parsing"))?;

    let normalized = normalize_numeric_context(text_sample);
    let mut items = Vec::new();

    for label in INDEX_LABELS {
        if let Some(index) = normalized.find(label) {
            let after = &normalized[index + label.len()..];
            if let Some(entry) = extract_index(label, after) {
                items.push(entry);
            }
        }
    }

    if items.is_empty() {
        bail!("unable to extract market indices from observation");
    }

    let output = MarketInfoOutput {
        schema: SCHEMA_ID,
        source_url: metadata.source_url,
        captured_at: metadata.captured_at,
        items,
    };

    serde_json::to_value(output).context("serialize market info output")
}

fn extract_index(label: &str, slice: &str) -> Option<MarketInfoItem> {
    let numbers: Vec<f64> = NUMBER_RE
        .find_iter(slice)
        .filter_map(|m| m.as_str().parse::<f64>().ok())
        .take(3)
        .collect();
    if numbers.len() < 3 {
        return None;
    }
    Some(MarketInfoItem {
        index_name: label.to_string(),
        value: numbers[0],
        change: numbers[1],
        change_pct: numbers[2],
    })
}

fn normalize_numeric_context(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut normalized = String::with_capacity(chars.len());

    for i in 0..chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() {
            let prev_numeric = i > 0 && is_numeric(chars[i - 1]);
            let next_numeric = i + 1 < chars.len() && is_numeric(chars[i + 1]);
            if prev_numeric && next_numeric {
                continue;
            }
            if !normalized.ends_with(' ') {
                normalized.push(' ');
            }
        } else {
            normalized.push(ch);
        }
    }

    normalized
}

fn is_numeric(ch: char) -> bool {
    ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+'
}
