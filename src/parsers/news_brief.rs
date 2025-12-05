use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;

const SCHEMA_ID: &str = "news_brief_v1";
const MAX_ITEMS: usize = 5;

#[derive(Debug, Serialize)]
struct NewsBriefItem {
    title: String,
    summary: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    published_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct NewsBriefOutput {
    schema: &'static str,
    items: Vec<NewsBriefItem>,
}

pub fn parse_news_brief(observation: &Value) -> Result<Value> {
    let data = observation.get("data").unwrap_or(observation);
    let mut items = Vec::new();
    items.extend(extract_from_links(data));
    if items.is_empty() {
        items.extend(extract_from_headings(data));
    }

    if items.is_empty() {
        bail!("unable to derive news entries from observation");
    }

    let trimmed: Vec<NewsBriefItem> = items.into_iter().take(MAX_ITEMS).collect();
    let output = NewsBriefOutput {
        schema: SCHEMA_ID,
        items: trimmed,
    };
    serde_json::to_value(output).context("serialize news brief output")
}

fn extract_from_links(data: &Value) -> Vec<NewsBriefItem> {
    let mut items = Vec::new();
    let Some(links) = data.get("links").and_then(Value::as_array) else {
        return items;
    };
    for link in links {
        let url = link
            .get("url")
            .or_else(|| link.get("href"))
            .and_then(Value::as_str);
        let title = link
            .get("text")
            .or_else(|| link.get("title"))
            .or_else(|| link.get("aria_label"))
            .and_then(Value::as_str);
        let summary = link
            .get("summary")
            .or_else(|| link.get("description"))
            .or_else(|| link.get("snippet"))
            .or_else(|| link.get("note"))
            .and_then(Value::as_str);
        let Some(url) = url else {
            continue;
        };
        let Some(title) = title else {
            continue;
        };
        let summary_text = summary
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or(title)
            .to_string();
        let source = link
            .get("source")
            .or_else(|| link.get("domain"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let published_at = link
            .get("published_at")
            .or_else(|| link.get("time"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        items.push(NewsBriefItem {
            title: title.trim().to_string(),
            summary: summary_text,
            url: url.to_string(),
            source,
            published_at,
        });
    }
    items
}

fn extract_from_headings(data: &Value) -> Vec<NewsBriefItem> {
    let mut items = Vec::new();
    let Some(headings) = data.get("headings").and_then(Value::as_array) else {
        return items;
    };
    let page_url = data
        .get("url")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "https://news.google.com".to_string());
    let paragraphs = data.get("paragraphs").and_then(Value::as_array);

    for (index, heading) in headings.iter().enumerate() {
        let Some(text) = heading.get("text").and_then(Value::as_str) else {
            continue;
        };
        let summary = paragraphs
            .and_then(|items| {
                items
                    .get(index)
                    .and_then(|p| p.get("text").and_then(Value::as_str))
            })
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| text.trim().to_string());
        let url = format!("{}#headline-{}", page_url, index + 1);
        items.push(NewsBriefItem {
            title: text.trim().to_string(),
            summary,
            url,
            source: None,
            published_at: None,
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_links_into_news_items() {
        let observation = json!({
            "data": {
                "links": [
                    {
                        "text": "Headline A",
                        "url": "https://news.example.com/a",
                        "summary": "Summary A",
                        "source": "Example News",
                        "published_at": "2025-11-22T08:20:00Z"
                    },
                    {
                        "text": "Headline B",
                        "url": "https://news.example.com/b",
                        "description": "Story B"
                    }
                ]
            }
        });
        let value = parse_news_brief(&observation).expect("news parse");
        assert_eq!(value.get("schema").and_then(Value::as_str), Some(SCHEMA_ID));
        let items = value.get("items").and_then(Value::as_array).expect("items");
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].get("summary").and_then(Value::as_str),
            Some("Summary A")
        );
        assert_eq!(
            items[1].get("summary").and_then(Value::as_str),
            Some("Story B")
        );
    }

    #[test]
    fn falls_back_to_headings() {
        let observation = json!({
            "data": {
                "url": "https://news.google.com",
                "headings": [
                    {"text": "Top headline"}
                ],
                "paragraphs": [
                    {"text": "Details about the top story."}
                ]
            }
        });
        let value = parse_news_brief(&observation).expect("news parse");
        let items = value.get("items").and_then(Value::as_array).unwrap();
        assert_eq!(items.len(), 1);
        let first = &items[0];
        assert_eq!(
            first.get("title").and_then(Value::as_str),
            Some("Top headline")
        );
        assert!(first
            .get("url")
            .and_then(Value::as_str)
            .unwrap()
            .starts_with("https://news.google.com"));
    }

    #[test]
    fn errors_when_no_data() {
        let observation = json!({ "data": { "headings": [], "links": [] } });
        assert!(parse_news_brief(&observation).is_err());
    }
}
