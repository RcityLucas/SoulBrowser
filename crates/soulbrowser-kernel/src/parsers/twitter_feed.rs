use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::parsers::{extract_observation_metadata, normalize_whitespace, text_from_candidates};

const SCHEMA_ID: &str = "twitter_feed_v1";
static HANDLE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)@([a-z0-9_]{1,30})").expect("handle regex"));

#[derive(Debug, Serialize)]
pub struct TwitterFeedItem {
    pub url: String,
    pub author_handle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TwitterFeedOutput {
    pub schema: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_handle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
    pub items: Vec<TwitterFeedItem>,
}

/// Parse generic observation output into normalized Twitter feed entries.
pub fn parse_twitter_feed(observation: &Value) -> Result<Value> {
    let metadata = extract_observation_metadata(observation);
    let data = observation.get("data").unwrap_or(observation);
    let mut items = gather_from_links(data);
    if items.is_empty() {
        items = gather_from_text_sample(observation);
    }
    items.sort_by(|a, b| a.url.cmp(&b.url));
    items.dedup_by(|a, b| a.url == b.url);

    if items.is_empty() {
        bail!("unable to extract tweets from observation");
    }

    let source_url = metadata.source_url.clone();
    let account_handle = profile_handle_from_url(source_url.as_deref())
        .or_else(|| items.first().map(|item| item.author_handle.clone()));
    let account_name = metadata.primary_title();

    let output = TwitterFeedOutput {
        schema: SCHEMA_ID,
        source_url,
        account_handle,
        account_name,
        items,
    };

    serde_json::to_value(output).context("serialize twitter feed output")
}

fn gather_from_links(data: &Value) -> Vec<TwitterFeedItem> {
    let mut items = Vec::new();
    let Some(links) = data.get("links").and_then(Value::as_array) else {
        return items;
    };

    for link in links {
        let Some(url) = link_url(link) else {
            continue;
        };
        if !is_twitter_status(&url) {
            continue;
        }
        let Some(content) = extract_text(link) else {
            continue;
        };
        let handle = link
            .get("author")
            .or_else(|| link.get("subtitle"))
            .and_then(Value::as_str)
            .and_then(|text| extract_handle(text))
            .or_else(|| twitter_handle_from_url(&url));
        let Some(author_handle) = handle else {
            continue;
        };
        let author_name = link
            .get("author")
            .or_else(|| link.get("subtitle"))
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .filter(|s| !s.starts_with('@'));
        let timestamp = link
            .get("published_at")
            .or_else(|| link.get("time"))
            .or_else(|| link.get("datetime"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());

        items.push(TwitterFeedItem {
            url,
            author_handle,
            author_name,
            content,
            timestamp,
        });
    }

    items
}

fn gather_from_text_sample(observation: &Value) -> Vec<TwitterFeedItem> {
    let mut items = Vec::new();
    let Some(sample) = observation.get("text_sample").and_then(Value::as_str) else {
        return items;
    };
    for line in sample.lines() {
        if let Some((handle, content)) = split_line(line) {
            items.push(TwitterFeedItem {
                url: observation
                    .get("url")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "https://twitter.com".to_string()),
                author_handle: handle,
                author_name: None,
                content: normalize_whitespace(&content),
                timestamp: None,
            });
        }
    }
    items
}

fn split_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let caps = HANDLE_RE.captures(trimmed)?;
    let handle = caps
        .get(1)
        .map(|m| format!("@{}", m.as_str().to_lowercase()))?;
    let idx = caps.get(0)?.end();
    let tail = trimmed[idx..].trim();
    if tail.is_empty() {
        return None;
    }
    Some((handle, tail.to_string()))
}

fn extract_text(link: &Value) -> Option<String> {
    text_from_candidates(
        link,
        &[
            "text",
            "title",
            "summary",
            "description",
            "snippet",
            "aria_label",
        ],
    )
}

fn link_url(link: &Value) -> Option<String> {
    text_from_candidates(link, &["url", "href"])
}

fn is_twitter_status(url: &str) -> bool {
    if let Ok(parsed) = Url::parse(url) {
        let host_matches = parsed
            .host_str()
            .map(|host| host.contains("twitter.com") || host.contains("x.com"))
            .unwrap_or(false);
        if host_matches {
            if let Some(mut segments) = parsed.path_segments() {
                if let Some(first) = segments.next() {
                    return !first.is_empty()
                        && segments.any(|seg| seg.eq_ignore_ascii_case("status"));
                }
            }
        }
    }
    false
}

fn twitter_handle_from_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let mut segments = parsed.path_segments()?;
    let first = segments.next()?;
    if first.is_empty() {
        return None;
    }
    Some(format!("@{}", first))
}

fn profile_handle_from_url(url: Option<&str>) -> Option<String> {
    let parsed = url.and_then(|u| Url::parse(u).ok())?;
    if !(parsed
        .host_str()
        .map(|host| host.contains("twitter.com") || host.contains("x.com"))
        .unwrap_or(false))
    {
        return None;
    }
    let mut segments = parsed.path_segments()?;
    let first = segments.next()?;
    if first.is_empty() || matches!(first, "home" | "explore") {
        return None;
    }
    Some(format!("@{}", first))
}

fn extract_handle(text: &str) -> Option<String> {
    HANDLE_RE
        .captures(text)
        .and_then(|caps| caps.get(1))
        .map(|m| format!("@{}", m.as_str().to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_links_into_items() {
        let observation = json!({
            "url": "https://twitter.com/soulbrowser",
            "data": {
                "links": [
                    {
                        "text": "Testing parser",
                        "url": "https://twitter.com/soulbrowser/status/1",
                        "author": "SoulBrowser",
                        "published_at": "2025-11-26T10:00:00Z"
                    },
                    {
                        "title": "Another tweet",
                        "url": "https://x.com/soulbrowser/status/2",
                        "subtitle": "@SoulBrowser · 3h"
                    }
                ]
            }
        });
        let value = parse_twitter_feed(&observation).expect("should parse");
        assert_eq!(value.get("schema").and_then(Value::as_str), Some(SCHEMA_ID));
        let items = value.get("items").and_then(Value::as_array).unwrap();
        assert_eq!(items.len(), 2);
        let first = &items[0];
        assert_eq!(
            first.get("author_handle").and_then(Value::as_str),
            Some("@soulbrowser")
        );
        assert_eq!(
            first.get("content").and_then(Value::as_str),
            Some("Testing parser")
        );
        assert_eq!(
            first.get("timestamp").and_then(Value::as_str),
            Some("2025-11-26T10:00:00Z")
        );
    }

    #[test]
    fn falls_back_to_text_sample() {
        let observation = json!({
            "url": "https://twitter.com/agent",
            "text_sample": "@agentAI Testing fallback parser"
        });
        let value = parse_twitter_feed(&observation).expect("fallback");
        let items = value.get("items").and_then(Value::as_array).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("content").and_then(Value::as_str),
            Some("Testing fallback parser")
        );
    }

    #[test]
    fn errors_when_no_tweets() {
        let observation = json!({ "text_sample": "no twitter data" });
        assert!(parse_twitter_feed(&observation).is_err());
    }
}
