use chrono::Utc;
use serde_json::Value;

/// Common metadata derived from an observation snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationMetadata {
    pub source_url: Option<String>,
    pub captured_at: Option<String>,
    pub hero_text: Option<String>,
    pub title: Option<String>,
    pub screenshot_path: Option<String>,
}

impl ObservationMetadata {
    /// Returns the most descriptive title available (hero text first, then the page title).
    pub fn primary_title(&self) -> Option<String> {
        self.hero_text
            .clone()
            .or_else(|| self.title.clone())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    }
}

/// Collect metadata fields frequently needed by parsers (source URL, timestamps, hero text).
pub fn extract_observation_metadata(observation: &Value) -> ObservationMetadata {
    ObservationMetadata {
        source_url: observation_source_url(observation),
        captured_at: observation_captured_at(observation),
        hero_text: text_from_candidates(observation, &["hero_text"]),
        title: text_from_candidates(observation, &["title"]),
        screenshot_path: text_from_candidates(observation, &["screenshot_path"]),
    }
}

/// Return the observation URL if available.
pub fn observation_source_url(observation: &Value) -> Option<String> {
    text_from_candidates(observation, &["url", "data.url"])
}

/// Return the observation timestamp (fetched_at/captured_at) or `None` if missing.
pub fn observation_captured_at(observation: &Value) -> Option<String> {
    text_from_candidates(
        observation,
        &["fetched_at", "captured_at", "timestamp", "observed_at"],
    )
}

/// Normalize whitespace inside text snippets by collapsing runs into single spaces.
pub fn normalize_whitespace(input: &str) -> String {
    input
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Pull the first non-empty string for any of the provided dotted paths.
pub fn text_from_candidates(value: &Value, fields: &[&str]) -> Option<String> {
    for field in fields {
        if let Some(text) = text_at_path(value, field) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn text_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a str> {
    let mut current = value;
    for key in path.split('.') {
        current = current.get(key)?;
    }
    current.as_str()
}

/// Build a timestamp string (RFC3339) for use in parser outputs that need `fetched_at` defaults.
#[allow(dead_code)]
pub fn now_timestamp() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_metadata_fields() {
        let observation = json!({
            "url": "https://example.com",
            "fetched_at": "2025-11-27T00:00:00Z",
            "hero_text": " Hero Title ",
            "title": "Fallback"
        });
        let metadata = extract_observation_metadata(&observation);
        assert_eq!(metadata.source_url.as_deref(), Some("https://example.com"));
        assert_eq!(
            metadata.captured_at.as_deref(),
            Some("2025-11-27T00:00:00Z")
        );
        assert_eq!(metadata.primary_title().as_deref(), Some("Hero Title"));
    }

    #[test]
    fn normalizes_whitespace() {
        let text = "First\nSecond\t\tThird";
        assert_eq!(normalize_whitespace(text), "First Second Third");
    }

    #[test]
    fn returns_first_matching_text_candidate() {
        let value = json!({
            "title": "",
            "hero_text": "Hero",
            "data": {"url": "https://example.com"}
        });
        assert_eq!(
            observation_source_url(&value).as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            text_from_candidates(&value, &["title", "hero_text"]).as_deref(),
            Some("Hero")
        );
    }

    #[test]
    fn timestamp_helper_returns_rfc3339() {
        let ts = now_timestamp();
        assert!(ts.contains('T'));
        assert!(ts.ends_with('Z'));
    }
}
