use std::collections::HashSet;

use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::parsers::{extract_observation_metadata, normalize_whitespace, text_from_candidates};

const SCHEMA_ID: &str = "hackernews_feed_v1";
static POINTS_AUTHOR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?P<points>\d+)\s+points?\s+by\s+(?P<author>[a-z0-9_\-]+)")
        .expect("points regex")
});
static COMMENTS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(?P<count>\d+)\s+comments?").expect("comments regex"));
static RANK_PREFIX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(?P<rank>\\d+)[\\.).\\s]+(?P<title>.+)$").expect("rank regex"));

#[derive(Debug, Serialize, Clone)]
pub struct HackerNewsFeedItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discussion_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_count: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct HackerNewsFeedOutput {
    pub schema: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_title: Option<String>,
    pub items: Vec<HackerNewsFeedItem>,
}

/// Parse observation output into the `hackernews_feed_v1` schema.
pub fn parse_hackernews_feed(observation: &Value) -> Result<Value> {
    let metadata = extract_observation_metadata(observation);
    let source_url = metadata.source_url.clone();
    let captured_at = metadata.captured_at.clone();
    let data = observation.get("data").unwrap_or(observation);
    let mut items = gather_from_links(data);
    if items.is_empty() {
        items = gather_from_text_sample(observation);
    }
    if items.is_empty() {
        bail!("unable to extract HackerNews stories from observation");
    }

    dedup_items(&mut items);
    enrich_with_metadata(&mut items, observation);

    let output = HackerNewsFeedOutput {
        schema: SCHEMA_ID,
        source_url,
        captured_at,
        page_title: metadata.primary_title(),
        items,
    };

    serde_json::to_value(output).context("serialize hackernews feed output")
}

fn gather_from_links(data: &Value) -> Vec<HackerNewsFeedItem> {
    let Some(links) = data.get("links").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut items: Vec<HackerNewsFeedItem> = Vec::new();
    let mut seen_urls = HashSet::new();

    for link in links {
        let Some(url) = link_url(link) else {
            continue;
        };
        let Some(text) = link_text(link) else {
            continue;
        };
        if is_discussion_link(&url, &text) {
            if let Some(last) = items.last_mut() {
                if last.discussion_url.is_none() {
                    last.discussion_url = Some(url.clone());
                }
                if last.comment_count.is_none() {
                    last.comment_count = parse_comment_count(&text);
                }
            }
            continue;
        }
        if !looks_like_story(&text, &url) {
            continue;
        }
        let title = strip_rank_prefix(&text);
        if title.is_empty() {
            continue;
        }
        if !seen_urls.insert(url.clone()) {
            continue;
        }
        let mut item = HackerNewsFeedItem {
            rank: parse_rank(&text),
            title: title.to_string(),
            url,
            discussion_url: None,
            points: None,
            author: None,
            comment_count: None,
        };
        if item.rank.is_none() {
            item.rank = Some((items.len() + 1) as u32);
        }
        items.push(item);
    }

    items
}

fn gather_from_text_sample(observation: &Value) -> Vec<HackerNewsFeedItem> {
    let Some(sample) = observation.get("text_sample").and_then(Value::as_str) else {
        return Vec::new();
    };
    let fallback_url = observation
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("https://news.ycombinator.com")
        .to_string();
    let mut items: Vec<HackerNewsFeedItem> = Vec::new();
    let mut current: Option<HackerNewsFeedItem> = None;

    for line in sample.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.to_lowercase().contains("points by") {
            if let Some(meta) = parse_meta_line(trimmed) {
                if let Some(item) = current.as_mut() {
                    if item.points.is_none() {
                        item.points = meta.points;
                    }
                    if item.author.is_none() {
                        item.author = meta.author;
                    }
                    if item.comment_count.is_none() {
                        item.comment_count = meta.comment_count;
                    }
                }
            }
            continue;
        }
        if trimmed.starts_with("http") {
            if let Some(item) = current.as_mut() {
                item.url = trimmed.to_string();
            }
            continue;
        }
        if trimmed.starts_with('(') && trimmed.ends_with(')') {
            continue;
        }
        let parsed_rank = parse_rank(trimmed);
        if parsed_rank.is_none()
            && !trimmed
                .chars()
                .next()
                .map(|ch| ch.is_ascii_digit())
                .unwrap_or(false)
        {
            continue;
        }
        if let Some(item) = current.take() {
            items.push(item);
        }
        let candidate = HackerNewsFeedItem {
            rank: parsed_rank.or_else(|| Some((items.len() + 1) as u32)),
            title: strip_rank_prefix(trimmed).to_string(),
            url: format!("{}#story-{}", fallback_url, items.len() + 1),
            discussion_url: None,
            points: None,
            author: None,
            comment_count: None,
        };
        if candidate.title.is_empty() {
            continue;
        }
        current = Some(candidate);
    }

    if let Some(item) = current.take() {
        items.push(item);
    }

    items
}

fn enrich_with_metadata(items: &mut [HackerNewsFeedItem], observation: &Value) {
    let mut meta_entries = metadata_entries(observation).into_iter();
    for item in items.iter_mut() {
        if let Some(meta) = meta_entries.next() {
            if let Some(points) = meta.points {
                item.points = Some(points);
            }
            if let Some(author) = meta.author.clone() {
                item.author = Some(author);
            }
            if let Some(comments) = meta.comment_count {
                item.comment_count = Some(comments);
            }
        }
    }
}

fn metadata_entries(observation: &Value) -> Vec<MetaEntry> {
    let mut entries = Vec::new();
    for container in [observation.get("data"), Some(observation)] {
        if let Some(value) = container {
            if let Some(paragraphs) = value.get("paragraphs").and_then(Value::as_array) {
                for paragraph in paragraphs {
                    if let Some(text) = paragraph.get("text").and_then(Value::as_str) {
                        if let Some(meta) = parse_meta_line(text) {
                            entries.push(meta);
                        }
                    }
                }
            }
        }
    }
    if entries.is_empty() {
        if let Some(sample) = observation.get("text_sample").and_then(Value::as_str) {
            for line in sample.lines() {
                if let Some(meta) = parse_meta_line(line) {
                    entries.push(meta);
                }
            }
        }
    }
    entries
}

fn parse_meta_line(line: &str) -> Option<MetaEntry> {
    let normalized = normalize_whitespace(line);
    let lower = normalized.to_lowercase();
    if !lower.contains("point") || !lower.contains("by") {
        return None;
    }
    let captures = POINTS_AUTHOR_RE.captures(&normalized);
    let points = captures
        .as_ref()
        .and_then(|caps| caps.name("points"))
        .and_then(|m| m.as_str().parse::<u32>().ok());
    let author = captures
        .as_ref()
        .and_then(|caps| caps.name("author"))
        .map(|m| m.as_str().trim().trim_matches('|').to_string());
    let comment_count = COMMENTS_RE
        .captures(&normalized)
        .and_then(|caps| caps.name("count"))
        .and_then(|m| m.as_str().parse::<u32>().ok())
        .or_else(|| {
            if lower.contains("discuss") {
                Some(0)
            } else {
                None
            }
        });
    if points.is_none() && author.is_none() && comment_count.is_none() {
        return None;
    }
    Some(MetaEntry {
        points,
        author,
        comment_count,
    })
}

fn parse_rank(text: &str) -> Option<u32> {
    RANK_PREFIX_RE
        .captures(text.trim())
        .and_then(|caps| caps.name("rank"))
        .and_then(|m| m.as_str().parse::<u32>().ok())
}

fn strip_rank_prefix(text: &str) -> &str {
    if let Some(caps) = RANK_PREFIX_RE.captures(text.trim()) {
        if let Some(title) = caps.name("title") {
            return title.as_str().trim();
        }
    }

    let trimmed = text.trim();
    let mut byte_index = 0;
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            byte_index += ch.len_utf8();
            continue;
        }
        break;
    }
    if byte_index > 0 {
        let remainder = trimmed[byte_index..]
            .trim_start_matches(|c: char| c == '.' || c == ')' || c == ':' || c.is_whitespace())
            .trim();
        if !remainder.is_empty() {
            return remainder;
        }
    }

    trimmed
}

fn link_url(value: &Value) -> Option<String> {
    text_from_candidates(value, &["url", "href"])
}

fn link_text(value: &Value) -> Option<String> {
    text_from_candidates(
        value,
        &["text", "title", "aria_label", "summary", "description"],
    )
    .map(|text| normalize_whitespace(&text))
    .filter(|text| !text.is_empty())
}

fn is_discussion_link(url: &str, text: &str) -> bool {
    match Url::parse(url) {
        Ok(parsed) => {
            let host = parsed.host_str().unwrap_or("").to_lowercase();
            if !host.contains("news.ycombinator.com") {
                return false;
            }
            let lowered = text.to_lowercase();
            lowered.contains("comment") || lowered.contains("discuss")
        }
        Err(_) => false,
    }
}

fn looks_like_story(text: &str, url: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lowered = trimmed.to_lowercase();
    const NAV_LINKS: &[&str] = &[
        "new",
        "past",
        "comments",
        "ask",
        "show",
        "jobs",
        "submit",
        "login",
        "apply",
        "faq",
        "guidelines",
        "contact",
        "security",
        "bookmarklet",
        "y combinator",
    ];
    if NAV_LINKS.contains(&lowered.as_str()) {
        return false;
    }
    match Url::parse(url) {
        Ok(parsed) => {
            let host = parsed.host_str().unwrap_or("").to_lowercase();
            if host.contains("news.ycombinator.com") {
                if lowered.contains("ask hn")
                    || lowered.contains("show hn")
                    || lowered.contains("tell hn")
                    || lowered.contains("launch hn")
                {
                    return true;
                }
                if lowered.contains("comment") || lowered.contains("discuss") || lowered == "hide" {
                    return false;
                }
                return parsed.path().contains("item")
                    && !lowered.contains("login")
                    && !lowered.contains("submit");
            }
            true
        }
        Err(_) => true,
    }
}

fn parse_comment_count(text: &str) -> Option<u32> {
    COMMENTS_RE
        .captures(text)
        .and_then(|caps| caps.name("count"))
        .and_then(|m| m.as_str().parse::<u32>().ok())
        .or_else(|| {
            if text.to_lowercase().contains("discuss") {
                Some(0)
            } else {
                None
            }
        })
}

fn dedup_items(items: &mut Vec<HackerNewsFeedItem>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.url.clone()));
    for (index, item) in items.iter_mut().enumerate() {
        if item.rank.is_none() {
            item.rank = Some((index + 1) as u32);
        }
    }
}

#[derive(Debug, Clone)]
struct MetaEntry {
    points: Option<u32>,
    author: Option<String>,
    comment_count: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_links_into_feed() {
        let observation = json!({
            "url": "https://news.ycombinator.com/",
            "data": {
                "links": [
                    {
                        "text": "1. Launch HN: Demo",
                        "url": "https://example.com/demo"
                    },
                    {
                        "text": "42 comments",
                        "url": "https://news.ycombinator.com/item?id=1"
                    },
                    {
                        "text": "Ask HN: Example question",
                        "url": "https://news.ycombinator.com/item?id=2"
                    },
                    {
                        "text": "discuss",
                        "url": "https://news.ycombinator.com/item?id=2"
                    }
                ],
                "paragraphs": [
                    {"text": "42 points by alice | 42 comments"},
                    {"text": "17 points by bob | discuss"}
                ]
            }
        });
        let value = parse_hackernews_feed(&observation).expect("should parse");
        assert_eq!(value.get("schema").and_then(Value::as_str), Some(SCHEMA_ID));
        let items = value.get("items").and_then(Value::as_array).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].get("rank").and_then(Value::as_u64), Some(1));
        assert_eq!(
            items[0].get("discussion_url").and_then(Value::as_str),
            Some("https://news.ycombinator.com/item?id=1")
        );
        assert_eq!(items[0].get("points").and_then(Value::as_u64), Some(42));
        assert_eq!(
            items[1].get("comment_count").and_then(Value::as_u64),
            Some(0)
        );
    }

    #[test]
    fn falls_back_to_text_sample() {
        let observation = json!({
            "url": "https://news.ycombinator.com/",
            "text_sample": "1. Show HN: Example Tool\n120 points by carol | 33 comments\nhttps://example.com/tool\n\n2. Ask HN: Anything\n58 points by dan | discuss\nhttps://news.ycombinator.com/item?id=99"
        });
        let value = parse_hackernews_feed(&observation).expect("fallback parse");
        let items = value.get("items").and_then(Value::as_array).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].get("title").and_then(Value::as_str),
            Some("Show HN: Example Tool")
        );
        assert_eq!(items[0].get("points").and_then(Value::as_u64), Some(120));
        assert_eq!(items[1].get("discussion_url").and_then(Value::as_str), None);
    }

    #[test]
    fn errors_when_no_items() {
        let observation = json!({"text_sample": "Nothing here"});
        assert!(parse_hackernews_feed(&observation).is_err());
    }

    #[test]
    fn metadata_entries_detects_paragraphs() {
        let observation = json!({
            "url": "https://news.ycombinator.com/",
            "data": {
                "paragraphs": [
                    {"text": "42 points by alice | 42 comments"},
                    {"text": "17 points by bob | discuss"}
                ]
            }
        });
        let entries = metadata_entries(&observation);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].points, Some(42));
        assert_eq!(entries[0].comment_count, Some(42));
        assert_eq!(entries[1].comment_count, Some(0));
    }
}
