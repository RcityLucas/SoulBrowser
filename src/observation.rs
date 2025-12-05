use serde_json::{Map, Value};

const CONSENT_HINTS: &[&str] = &[
    "before you continue",
    "consent",
    "我们会使用 cookie",
    "使用 cookie",
    "接受全部",
];
const CAPTCHA_HINTS: &[&str] = &[
    "captcha",
    "验证码",
    "are you a robot",
    "human verification",
    "検証",
];
const TRAFFIC_HINTS: &[&str] = &[
    "unusual traffic",
    "automated queries",
    "unusual activity",
    "我们无法处理您的请求",
];
const LOGIN_HINTS: &[&str] = &["log in", "登录", "sign in", "account required"];

pub fn annotate_obstruction(value: &mut Value) {
    if let Some(kind) = detect_obstruction(value) {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("obstruction_type".to_string(), Value::String(kind));
        }
    }
    attach_dom_statistics(value);
}

pub fn obstruction_from_entry(entry: &Value) -> Option<String> {
    entry
        .get("obstruction_type")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| detect_obstruction(entry))
}

fn detect_obstruction(value: &Value) -> Option<String> {
    let (blob, url) = aggregate_text(value);
    if blob.is_empty() && url.is_none() {
        return None;
    }
    if contains_any(&blob, CONSENT_HINTS) {
        return Some("consent_gate".to_string());
    }
    if contains_any(&blob, CAPTCHA_HINTS) {
        return Some("captcha".to_string());
    }
    if contains_any(&blob, TRAFFIC_HINTS) {
        return Some("unusual_traffic".to_string());
    }
    if contains_any(&blob, LOGIN_HINTS) {
        return Some("login_wall".to_string());
    }
    if blob.trim().is_empty() {
        if let Some(url) = url {
            if url.contains("about:blank") {
                return Some("blank_page".to_string());
            }
        }
    }
    if let Some(stats) = value.get("dom_statistics").and_then(Value::as_object) {
        let interactive = stats
            .get("interactive_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let headings = stats
            .get("heading_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let paragraphs = stats
            .get("paragraph_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if interactive == 0 && headings == 0 && paragraphs == 0 {
            return Some("blank_page".to_string());
        }
    }
    None
}

fn attach_dom_statistics(value: &mut Value) {
    let Some(stats) = compute_dom_statistics(value) else {
        return;
    };
    if let Some(obj) = value.as_object_mut() {
        obj.insert("dom_statistics".to_string(), Value::Object(stats));
    }
}

fn compute_dom_statistics(value: &Value) -> Option<Map<String, Value>> {
    let source = value.get("data").unwrap_or(value);
    let mut stats = Map::new();

    let link_count = insert_array_count(&mut stats, "link_count", source.get("links"));
    insert_array_count(&mut stats, "heading_count", source.get("headings"));
    insert_array_count(&mut stats, "paragraph_count", source.get("paragraphs"));
    insert_array_count(&mut stats, "key_value_count", source.get("key_values"));
    insert_array_count(&mut stats, "image_count", source.get("images"));
    insert_array_count(&mut stats, "table_count", source.get("tables"));
    let button_count = insert_array_count(&mut stats, "button_count", source.get("buttons"));
    let input_count = insert_array_count(&mut stats, "input_count", source.get("inputs"));
    insert_array_count(&mut stats, "form_count", source.get("forms"));
    insert_array_count(
        &mut stats,
        "scroll_container_count",
        source.get("scroll_containers"),
    );

    let interactive_sum = link_count + button_count + input_count;
    if interactive_sum > 0 {
        stats.insert(
            "interactive_count".to_string(),
            Value::Number((interactive_sum as u64).into()),
        );
    }

    if let Some(len) = source
        .get("text_sample_length")
        .and_then(Value::as_u64)
        .or_else(|| {
            source
                .get("text_sample")
                .and_then(Value::as_str)
                .map(|sample| sample.chars().count() as u64)
        })
    {
        if len > 0 {
            stats.insert("text_sample_length".to_string(), Value::Number(len.into()));
        }
    }

    if stats.is_empty() {
        None
    } else {
        Some(stats)
    }
}

fn insert_array_count(stats: &mut Map<String, Value>, label: &str, node: Option<&Value>) -> u64 {
    let Some(array) = node.and_then(Value::as_array) else {
        return 0;
    };
    let len = array.len() as u64;
    if len > 0 {
        stats.insert(label.to_string(), Value::Number(len.into()));
    }
    len
}

fn aggregate_text(value: &Value) -> (String, Option<String>) {
    if let Some(data) = value.get("data") {
        return collect_fields(data);
    }
    collect_fields(value)
}

fn collect_fields(node: &Value) -> (String, Option<String>) {
    let mut parts = Vec::new();
    if let Some(identity) = node.get("identity").and_then(Value::as_str) {
        parts.push(identity.trim().to_string());
    }
    if let Some(text) = node.get("text_sample").and_then(Value::as_str) {
        parts.push(text.trim().to_string());
    }
    if let Some(note) = node.get("note").and_then(Value::as_str) {
        parts.push(note.trim().to_string());
    }
    if let Some(headings) = node.get("headings").and_then(Value::as_array) {
        for heading in headings {
            if let Some(text) = heading.get("text").and_then(Value::as_str) {
                parts.push(text.trim().to_string());
            }
        }
    }
    let url = node
        .get("url")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    (parts.join(" "), url)
}

fn contains_any(blob: &str, needles: &[&str]) -> bool {
    if blob.is_empty() {
        return false;
    }
    let lowercase = blob.to_ascii_lowercase();
    needles.iter().any(|needle| {
        let needle_lower = needle.to_ascii_lowercase();
        lowercase.contains(&needle_lower) || blob.contains(needle)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_captcha() {
        let mut value = json!({
            "data": {
                "identity": "Google",
                "text_sample": "Please complete the CAPTCHA before continuing"
            }
        });
        annotate_obstruction(&mut value);
        assert_eq!(
            value
                .get("obstruction_type")
                .and_then(Value::as_str)
                .unwrap(),
            "captcha"
        );
    }

    #[test]
    fn attaches_dom_statistics() {
        let mut value = json!({
            "data": {
                "links": [
                    {"href": "https://example.com", "text": "Example"},
                    {"href": "https://example.org", "text": "Org"}
                ],
                "headings": [
                    {"level": "h1", "text": "Title"}
                ],
                "text_sample": "Lorem ipsum"
            }
        });
        annotate_obstruction(&mut value);
        let stats = value
            .get("dom_statistics")
            .and_then(Value::as_object)
            .expect("dom stats");
        assert_eq!(stats.get("link_count").and_then(Value::as_u64), Some(2));
        assert_eq!(stats.get("heading_count").and_then(Value::as_u64), Some(1));
        assert_eq!(
            stats.get("text_sample_length").and_then(Value::as_u64),
            Some(11)
        );
    }
}
