use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use url::Url;

const SCHEMA_ID: &str = "facebook_feed_v1";
static AUTHOR_CLEAN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(shared a memory|shared a post|updated their status)").expect("regex")
});

#[derive(Debug, Serialize)]
pub struct FacebookFeedItem {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_profile: Option<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FacebookFeedOutput {
    pub schema: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_name: Option<String>,
    pub items: Vec<FacebookFeedItem>,
}

/// Parse observation output into Facebook timeline entries.
pub fn parse_facebook_feed(observation: &Value) -> Result<Value> {
    let data = observation.get("data").unwrap_or(observation);
    let mut items = gather_from_links(data);
    if items.is_empty() {
        items = gather_from_paragraphs(data, observation);
    }
    items.sort_by(|a, b| a.url.cmp(&b.url));
    items.dedup_by(|a, b| a.url == b.url && a.content == b.content);

    if items.is_empty() {
        bail!("unable to extract facebook posts from observation");
    }

    let source_url = observation
        .get("url")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let page_name = observation
        .get("hero_text")
        .or_else(|| observation.get("title"))
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let output = FacebookFeedOutput {
        schema: SCHEMA_ID,
        source_url,
        page_name,
        items,
    };

    serde_json::to_value(output).context("serialize facebook feed output")
}

fn gather_from_links(data: &Value) -> Vec<FacebookFeedItem> {
    let mut items = Vec::new();
    let Some(links) = data.get("links").and_then(Value::as_array) else {
        return items;
    };

    for link in links {
        let Some(url) = link_url(link) else {
            continue;
        };
        if !is_facebook_post(&url) {
            continue;
        }
        let Some(content) = extract_text(link) else {
            continue;
        };
        let author_name = link
            .get("author")
            .or_else(|| link.get("subtitle"))
            .or_else(|| link.get("aria_label"))
            .and_then(Value::as_str)
            .map(|s| clean_author(s));
        let author_profile = link
            .get("author_url")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .or_else(|| profile_from_url(&url));
        let timestamp = link
            .get("published_at")
            .or_else(|| link.get("time"))
            .or_else(|| link.get("datetime"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        items.push(FacebookFeedItem {
            url,
            author_name,
            author_profile,
            content,
            timestamp,
        });
    }

    items
}

fn gather_from_paragraphs(data: &Value, observation: &Value) -> Vec<FacebookFeedItem> {
    let mut items = Vec::new();
    let Some(paragraphs) = data
        .get("paragraphs")
        .or_else(|| observation.get("paragraphs"))
        .and_then(Value::as_array)
    else {
        return items;
    };

    for (idx, para) in paragraphs.iter().enumerate() {
        let Some(text) = para.get("text").and_then(Value::as_str) else {
            continue;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let fallback_url = observation
            .get("url")
            .and_then(Value::as_str)
            .map(|s| format!("{}#post-{}", s, idx + 1))
            .unwrap_or_else(|| "https://www.facebook.com".to_string());
        items.push(FacebookFeedItem {
            url: fallback_url,
            author_name: observation
                .get("hero_text")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string()),
            author_profile: observation
                .get("url")
                .and_then(Value::as_str)
                .map(|s| s.to_string()),
            content: trimmed.to_string(),
            timestamp: None,
        });
    }
    items
}

fn extract_text(entry: &Value) -> Option<String> {
    let candidates = ["text", "title", "summary", "description", "snippet"];
    for field in candidates {
        if let Some(value) = entry.get(field).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn link_url(entry: &Value) -> Option<String> {
    entry
        .get("url")
        .or_else(|| entry.get("href"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}

fn is_facebook_post(url: &str) -> bool {
    if let Ok(parsed) = Url::parse(url) {
        let host_ok = parsed
            .host_str()
            .map(|host| host.contains("facebook.com") || host.contains("fb.com"))
            .unwrap_or(false);
        if !host_ok {
            return false;
        }
        if let Some(path) = parsed.path_segments() {
            let joined = path.collect::<Vec<_>>().join("/");
            return joined.contains("posts")
                || joined.contains("videos")
                || joined.contains("photos")
                || joined.contains("story.php");
        }
    }
    false
}

fn profile_from_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let mut segments = parsed.path_segments()?;
    let first = segments.next()?;
    if first.is_empty() || first.eq_ignore_ascii_case("story.php") {
        return None;
    }
    let profile_url = format!("https://{}/{}", parsed.host_str()?, first);
    Some(profile_url)
}

fn clean_author(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    let lowered = AUTHOR_CLEAN_RE.replace(trimmed, "");
    lowered.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_links_into_facebook_items() {
        let observation = json!({
            "url": "https://www.facebook.com/soulbrowser",
            "data": {
                "links": [
                    {
                        "text": "We just shipped a new release",
                        "url": "https://www.facebook.com/soulbrowser/posts/1",
                        "author": "Soul Browser",
                        "published_at": "2025-11-26T10:00:00Z"
                    },
                    {
                        "title": "Launch recap",
                        "url": "https://www.facebook.com/soulbrowser/posts/2",
                        "subtitle": "Soul Browser shared a post"
                    }
                ]
            }
        });
        let value = parse_facebook_feed(&observation).expect("should parse");
        assert_eq!(value.get("schema").and_then(Value::as_str), Some(SCHEMA_ID));
        let items = value.get("items").and_then(Value::as_array).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].get("content").and_then(Value::as_str),
            Some("We just shipped a new release")
        );
        assert_eq!(
            items[0].get("author_name").and_then(Value::as_str),
            Some("Soul Browser")
        );
    }

    #[test]
    fn falls_back_to_paragraphs() {
        let observation = json!({
            "url": "https://www.facebook.com/soulbrowser",
            "hero_text": "Soul Browser",
            "paragraphs": [
                {"text": "Hello followers!"}
            ]
        });
        let value = parse_facebook_feed(&observation).expect("fallback");
        let items = value.get("items").and_then(Value::as_array).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("content").and_then(Value::as_str),
            Some("Hello followers!")
        );
    }

    #[test]
    fn errors_when_no_posts() {
        let observation = json!({ "data": { "links": [] } });
        assert!(parse_facebook_feed(&observation).is_err());
    }
}
