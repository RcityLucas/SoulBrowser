use anyhow::{anyhow, bail, Result};
use serde_json::Value;

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
        "news_brief_v1" => validate_news_brief(value),
        "github_repos_v1" => validate_github_repos(value),
        "twitter_feed_v1" => validate_twitter_feed(value),
        "facebook_feed_v1" => validate_facebook_feed(value),
        "hackernews_feed_v1" => validate_hackernews_feed(value),
        _ => Ok(()),
    }
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
}
