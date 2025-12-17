use std::collections::HashSet;

use anyhow::{anyhow, bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::parsers::{
    extract_observation_metadata, normalize_whitespace, text_from_candidates, ObservationMetadata,
};

const SCHEMA_ID: &str = "linkedin_profile_v1";
const MAX_ACTIVITY_ITEMS: usize = 6;
static COUNT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(\d+(?:[\.,]\d+)?)\s*([kmb])?").expect("count regex for linkedin parser")
});
const GENERIC_LINE_HINTS: &[&str] = &[
    "linkedin",
    "sign in",
    "sign up",
    "join now",
    "navigation menu",
    "search code",
    "log in",
    "contact linkedin",
];
const CONTACT_HINTS: &[&str] = &[
    "contact", "email", "website", "site", "phone", "reach", "联系", "邮箱",
];
const RECENT_ACTIVITY_HINTS: &[&str] = &[
    "post", "activity", "shared", "update", "article", "发布", "分享", "动态",
];

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LinkedInEntityType {
    Profile,
    Company,
    School,
    Unknown,
}

impl Default for LinkedInEntityType {
    fn default() -> Self {
        LinkedInEntityType::Profile
    }
}

#[derive(Debug, Serialize)]
struct LinkedInActivityItem {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, Serialize)]
struct LinkedInProfileOutput {
    schema: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    captured_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screenshot_path: Option<String>,
    entity_type: LinkedInEntityType,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    headline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    industry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    follower_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    employee_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    about: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    contact_urls: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    recent_activity: Vec<LinkedInActivityItem>,
}

/// Parse observation output into the `linkedin_profile_v1` schema.
pub fn parse_linkedin_profile(observation: &Value) -> Result<Value> {
    let metadata = extract_observation_metadata(observation);
    let data = observation.get("data").unwrap_or(observation);
    let profile_url = metadata.source_url.clone();
    let entity_type = infer_entity_type(profile_url.as_deref(), data);
    let name = extract_profile_name(data, &metadata)
        .ok_or_else(|| anyhow!("unable to determine LinkedIn profile/company name"))?;
    let headline = extract_headline(data, &metadata, &name);
    let industry = value_from_key_values(data, &["industry", "领域"]);
    let location = extract_location(data);
    let follower_count = parse_numeric_field(data, &["follower", "followers", "关注"]);
    let employee_count = parse_numeric_field(data, &["employee", "headcount", "员工"]);
    let about = extract_about_section(data);
    let contact_urls = extract_contact_urls(data);
    let recent_activity = extract_recent_activity(data);

    if headline.is_none()
        && about.is_none()
        && location.is_none()
        && follower_count.is_none()
        && employee_count.is_none()
        && recent_activity.is_empty()
    {
        bail!("LinkedIn observation lacks structured profile fields");
    }

    let output = LinkedInProfileOutput {
        schema: SCHEMA_ID,
        profile_url,
        captured_at: metadata.captured_at,
        screenshot_path: metadata.screenshot_path,
        entity_type,
        name,
        headline,
        industry,
        location,
        follower_count,
        employee_count,
        about,
        contact_urls,
        recent_activity,
    };

    serde_json::to_value(output).context("serialize linkedin profile output")
}

fn extract_profile_name(data: &Value, metadata: &ObservationMetadata) -> Option<String> {
    if let Some(hero) = metadata.hero_text.as_deref() {
        for line in hero_lines(hero) {
            if let Some(candidate) = sanitize_line(&line) {
                return Some(candidate);
            }
        }
    }
    if let Some(identity) = text_from_candidates(data, &["identity"]) {
        if let Some(candidate) = sanitize_line(&identity) {
            return Some(candidate);
        }
    }
    if let Some(headings) = data.get("headings").and_then(Value::as_array) {
        for heading in headings {
            let Some(text) = heading.get("text").and_then(Value::as_str) else {
                continue;
            };
            if let Some(level) = heading.get("level").and_then(Value::as_str) {
                if !matches!(level, "h1" | "h2" | "h3") {
                    continue;
                }
            }
            if let Some(candidate) = sanitize_line(text) {
                return Some(candidate);
            }
        }
    }
    if let Some(title) = metadata.title.as_deref() {
        let cleaned = clean_title(title);
        if let Some(candidate) = sanitize_line(&cleaned) {
            return Some(candidate);
        }
    }
    None
}

fn extract_headline(data: &Value, metadata: &ObservationMetadata, name: &str) -> Option<String> {
    if let Some(hero) = metadata.hero_text.as_deref() {
        let mut lines = hero_lines(hero).into_iter();
        if let Some(first) = lines.next() {
            if normalize_whitespace(&first) != name {
                if let Some(value) = sanitize_line(&first) {
                    return Some(value);
                }
            }
        }
        for line in lines {
            if let Some(value) = sanitize_line(&line) {
                if value != name {
                    return Some(value);
                }
            }
        }
    }
    if let Some(value) = value_from_key_values(data, &["headline", "tagline", "slogan"]) {
        return Some(value);
    }
    if let Some(title) = metadata.title.as_deref() {
        let cleaned = clean_title(title);
        let lower_name = name.to_ascii_lowercase();
        if let Some(idx) = cleaned.to_ascii_lowercase().find(&lower_name) {
            let tail = cleaned[idx + name.len()..]
                .trim_matches(|c: char| c == '-' || c == '|' || c == '·' || c == '•' || c == ' ')
                .trim();
            if !tail.is_empty() && !tail.eq_ignore_ascii_case(name) {
                return Some(tail.to_string());
            }
        } else if !cleaned.is_empty() && !cleaned.eq_ignore_ascii_case(name) {
            return Some(cleaned);
        }
    }
    None
}

fn extract_location(data: &Value) -> Option<String> {
    if let Some(value) = value_from_key_values(data, &["location", "地区", "headquarters", "总部"])
    {
        return Some(value);
    }
    value_from_paragraphs(data, &["based in", "总部", "headquartered", "位于"])
}

fn extract_about_section(data: &Value) -> Option<String> {
    let Some(paragraphs) = data.get("paragraphs").and_then(Value::as_array) else {
        return None;
    };
    for paragraph in paragraphs {
        if let Some(text) = paragraph_text(paragraph) {
            let normalized = normalize_whitespace(&text);
            let lower = normalized.to_ascii_lowercase();
            if normalized.len() < 60 {
                continue;
            }
            if let Some(_hint) = GENERIC_LINE_HINTS
                .iter()
                .find(|hint| lower.contains(**hint))
            {
                continue;
            }
            if lower.contains("log in") || lower.contains("block users") {
                continue;
            }
            return Some(normalized);
        }
    }
    None
}

fn extract_contact_urls(data: &Value) -> Vec<String> {
    let Some(links) = data.get("links").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut urls = Vec::new();
    for link in links {
        let Some(url) = link_url(link) else {
            continue;
        };
        if is_contact_link(&url, link) {
            urls.push(url);
        }
    }
    urls.sort();
    urls.dedup();
    urls
}

fn extract_recent_activity(data: &Value) -> Vec<LinkedInActivityItem> {
    let Some(links) = data.get("links").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    for link in links {
        let url = link_url(link);
        let text = text_from_candidates(
            link,
            &[
                "text",
                "title",
                "summary",
                "description",
                "aria_label",
                "snippet",
            ],
        );
        if !looks_like_activity(url.as_deref(), text.as_deref()) {
            continue;
        }
        let title = text.unwrap_or_else(|| url.clone().unwrap_or_default());
        let normalized = normalize_whitespace(&title);
        if normalized.is_empty() {
            continue;
        }
        let key = format!(
            "{}|{}",
            url.clone().unwrap_or_else(|| "unknown".to_string()),
            normalized
        );
        if !seen.insert(key) {
            continue;
        }
        let timestamp = link
            .get("published_at")
            .or_else(|| link.get("datetime"))
            .or_else(|| link.get("time"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let description = link
            .get("summary")
            .or_else(|| link.get("description"))
            .and_then(Value::as_str)
            .map(|s| normalize_whitespace(s));
        items.push(LinkedInActivityItem {
            title: normalized,
            url,
            timestamp,
            description,
        });
        if items.len() >= MAX_ACTIVITY_ITEMS {
            break;
        }
    }

    items
}

fn hero_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| normalize_whitespace(line))
        .filter(|line| !line.is_empty())
        .collect()
}

fn clean_title(title: &str) -> String {
    let replaced = title
        .replace("| LinkedIn", "")
        .replace("- LinkedIn", "")
        .replace("| 专业档案 | LinkedIn", "");
    normalize_whitespace(&replaced)
}

fn sanitize_line(text: &str) -> Option<String> {
    let normalized = normalize_whitespace(text);
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();
    if GENERIC_LINE_HINTS.iter().any(|hint| lower.contains(hint)) {
        return None;
    }
    Some(normalized)
}

fn paragraph_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    value
        .get("text")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}

fn value_from_key_values(data: &Value, keywords: &[&str]) -> Option<String> {
    let Some(entries) = data.get("key_values").and_then(Value::as_array) else {
        return None;
    };
    let lowered_keywords: Vec<String> = keywords.iter().map(|kw| kw.to_ascii_lowercase()).collect();
    for entry in entries {
        let label = entry
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if lowered_keywords.iter().any(|kw| label.contains(kw)) {
            if let Some(value) = entry.get("value").and_then(Value::as_str) {
                let normalized = normalize_whitespace(value);
                if !normalized.is_empty() {
                    return Some(normalized);
                }
            }
        }
    }
    None
}

fn value_from_paragraphs(data: &Value, keywords: &[&str]) -> Option<String> {
    let Some(items) = data.get("paragraphs").and_then(Value::as_array) else {
        return None;
    };
    let lowered_keywords: Vec<String> = keywords.iter().map(|kw| kw.to_ascii_lowercase()).collect();
    for paragraph in items {
        if let Some(text) = paragraph_text(paragraph) {
            let normalized = normalize_whitespace(&text);
            let lower = normalized.to_ascii_lowercase();
            if lowered_keywords.iter().any(|kw| lower.contains(kw)) {
                return Some(normalized);
            }
        }
    }
    None
}

fn parse_numeric_field(data: &Value, keywords: &[&str]) -> Option<u64> {
    let mut candidates = Vec::new();
    if let Some(value) = value_from_key_values(data, keywords) {
        candidates.push(value);
    }
    if let Some(value) = value_from_counters(data, keywords) {
        candidates.push(value);
    }
    for candidate in candidates {
        if let Some(parsed) = parse_count(&candidate) {
            return Some(parsed);
        }
    }
    None
}

fn value_from_counters(data: &Value, keywords: &[&str]) -> Option<String> {
    let Some(items) = data.get("counters").and_then(Value::as_array) else {
        return None;
    };
    let lowered_keywords: Vec<String> = keywords.iter().map(|kw| kw.to_ascii_lowercase()).collect();
    for counter in items {
        let label = counter
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if lowered_keywords.iter().any(|kw| label.contains(kw)) {
            if let Some(value) = counter.get("value").and_then(Value::as_str) {
                let normalized = normalize_whitespace(value);
                if !normalized.is_empty() {
                    return Some(normalized);
                }
            }
        }
    }
    None
}

fn parse_count(text: &str) -> Option<u64> {
    let caps = COUNT_RE.captures(text.trim())?;
    let raw = caps.get(1)?.as_str();
    let mut normalized = String::new();
    let mut seen_decimal = false;
    let comma_sections: Vec<&str> = raw.split(',').collect();
    let treat_comma_as_decimal = raw.contains(',')
        && !raw.contains('.')
        && comma_sections.len() == 2
        && comma_sections
            .get(1)
            .map(|section| section.chars().all(|c| c.is_ascii_digit()) && section.len() <= 2)
            .unwrap_or(false);
    for ch in raw.chars() {
        if ch.is_ascii_digit() {
            normalized.push(ch);
        } else if ch == '.' && !seen_decimal {
            normalized.push('.');
            seen_decimal = true;
        } else if ch == ',' {
            if treat_comma_as_decimal && !seen_decimal {
                normalized.push('.');
                seen_decimal = true;
            } else {
                continue;
            }
        }
    }
    if normalized.is_empty() {
        return None;
    }
    let mut value: f64 = normalized.parse().ok()?;
    if let Some(suffix) = caps.get(2).map(|m| m.as_str().to_ascii_lowercase()) {
        value *= match suffix.as_str() {
            "k" => 1_000.0,
            "m" => 1_000_000.0,
            "b" => 1_000_000_000.0,
            _ => 1.0,
        };
    }
    Some(value.round() as u64)
}

fn link_url(link: &Value) -> Option<String> {
    text_from_candidates(link, &["url", "href"])
}

fn looks_like_activity(url: Option<&str>, text: Option<&str>) -> bool {
    if let Some(url) = url {
        if is_activity_url(url) {
            return true;
        }
    }
    if let Some(text) = text {
        let lower = text.to_ascii_lowercase();
        if RECENT_ACTIVITY_HINTS
            .iter()
            .any(|hint| lower.contains(hint))
        {
            return true;
        }
    }
    false
}

fn is_activity_url(url: &str) -> bool {
    if let Ok(parsed) = Url::parse(url) {
        if let Some(host) = parsed.host_str() {
            if !(host.contains("linkedin.com") || host.contains("lnkd.in")) {
                return false;
            }
        }
        let path = parsed.path().to_ascii_lowercase();
        return path.contains("/posts/")
            || path.contains("/feed/update")
            || path.contains("/pulse/")
            || path.contains("/events/");
    }
    false
}

fn is_contact_link(url: &str, link: &Value) -> bool {
    if url.starts_with("mailto:") || url.starts_with("tel:") {
        return true;
    }
    if let Ok(parsed) = Url::parse(url) {
        if let Some(host) = parsed.host_str() {
            if !(host.contains("linkedin.com") || host.contains("lnkd.in")) {
                return true;
            }
        }
    }
    let text_blob =
        text_from_candidates(link, &["text", "title", "aria_label", "subtitle", "note"])
            .unwrap_or_default()
            .to_ascii_lowercase();
    CONTACT_HINTS
        .iter()
        .any(|hint| text_blob.contains(&hint.to_ascii_lowercase()))
}

fn infer_entity_type(url: Option<&str>, data: &Value) -> LinkedInEntityType {
    if let Some(url) = url {
        let lowered = url.to_ascii_lowercase();
        if lowered.contains("/company/") {
            return LinkedInEntityType::Company;
        }
        if lowered.contains("/school/") {
            return LinkedInEntityType::School;
        }
        if lowered.contains("/in/") || lowered.contains("/pub/") {
            return LinkedInEntityType::Profile;
        }
    }
    if let Some(identity) = text_from_candidates(data, &["identity"]) {
        let lower = identity.to_ascii_lowercase();
        if lower.contains("company") || lower.contains("inc") {
            return LinkedInEntityType::Company;
        }
        if lower.contains("school") {
            return LinkedInEntityType::School;
        }
    }
    LinkedInEntityType::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_profile_observation() {
        let observation = json!({
            "url": "https://www.linkedin.com/in/jane-doe/",
            "hero_text": "Jane Doe\nStaff Engineer at Example",
            "captured_at": "2025-11-30T00:00:00Z",
            "data": {
                "identity": "Jane Doe",
                "paragraphs": [
                    "Experienced engineer shipping automation platforms and mentoring distributed teams across the globe."
                ],
                "key_values": [
                    {"label": "Location", "value": "San Francisco Bay Area"},
                    {"label": "Followers", "value": "12,345 followers"},
                    {"label": "Industry", "value": "Computer Software"}
                ],
                "links": [
                    {
                        "text": "Shared a post about launching SoulBrowser",
                        "url": "https://www.linkedin.com/feed/update/urn:li:activity:123",
                        "published_at": "2025-11-28T10:00:00Z",
                        "description": "Launch week recap"
                    },
                    {"text": "Personal website", "url": "https://janedoe.dev"},
                    {"text": "Contact Jane", "url": "mailto:jane@example.com"}
                ],
                "headings": [
                    {"level": "h1", "text": "Jane Doe"},
                    {"level": "h2", "text": "Staff Engineer"}
                ]
            }
        });

        let value = parse_linkedin_profile(&observation).expect("parse profile");
        assert_eq!(value.get("schema").and_then(Value::as_str), Some(SCHEMA_ID));
        assert_eq!(
            value.get("entity_type").and_then(Value::as_str),
            Some("profile")
        );
        assert_eq!(value.get("name").and_then(Value::as_str), Some("Jane Doe"));
        assert_eq!(
            value.get("headline").and_then(Value::as_str),
            Some("Staff Engineer at Example")
        );
        assert_eq!(
            value.get("location").and_then(Value::as_str),
            Some("San Francisco Bay Area")
        );
        assert_eq!(
            value.get("follower_count").and_then(Value::as_u64),
            Some(12_345)
        );
        assert!(value
            .get("recent_activity")
            .and_then(Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false));
        assert_eq!(
            value
                .get("contact_urls")
                .and_then(Value::as_array)
                .map(|items| items.len()),
            Some(2)
        );
    }

    #[test]
    fn parses_company_observation() {
        let observation = json!({
            "url": "https://www.linkedin.com/company/soulbrowser/",
            "title": "SoulBrowser - AI Automation | LinkedIn",
            "data": {
                "headings": [
                    {"level": "h1", "text": "SoulBrowser"},
                    {"level": "h2", "text": "AI Automation"}
                ],
                "paragraphs": [
                    "SoulBrowser builds deterministic browsing agents for enterprise research teams, combining Chrome CDP automation with schema-first deliveries."
                ],
                "key_values": [
                    {"label": "Headquarters", "value": "New York City"},
                    {"label": "Employees", "value": "200"}
                ],
                "counters": [
                    {"label": "Followers 8,901", "value": "8,901"}
                ],
                "links": [
                    {
                        "text": "Company update: launching flexible parsers",
                        "url": "https://www.linkedin.com/posts/soulbrowser_launch",
                        "time": "2025-11-21T12:00:00Z"
                    },
                    {
                        "text": "Visit website",
                        "url": "https://soulbrowser.ai"
                    }
                ]
            }
        });

        let value = parse_linkedin_profile(&observation).expect("parse company");
        assert_eq!(
            value.get("entity_type").and_then(Value::as_str),
            Some("company")
        );
        assert_eq!(
            value.get("employee_count").and_then(Value::as_u64),
            Some(200)
        );
        assert_eq!(
            value.get("follower_count").and_then(Value::as_u64),
            Some(8_901)
        );
        assert_eq!(
            value
                .get("about")
                .and_then(Value::as_str)
                .unwrap()
                .contains("agents"),
            true
        );
        assert!(value
            .get("contact_urls")
            .and_then(Value::as_array)
            .map(|items| items[0].as_str().unwrap().contains("soulbrowser"))
            .unwrap_or(false));
    }

    #[test]
    fn errors_without_basic_fields() {
        let observation = json!({
            "url": "https://www.linkedin.com/in/unknown"
        });
        assert!(parse_linkedin_profile(&observation).is_err());
    }
}
