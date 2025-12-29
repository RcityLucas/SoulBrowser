use crate::model::AgentRequest;
use once_cell::sync::Lazy;
use regex::Regex;
use url::form_urlencoded;

pub fn weather_query_text(request: &AgentRequest) -> String {
    let mut sources = Vec::new();
    if let Some(goal) = request.intent.primary_goal.as_deref() {
        sources.push(goal);
    }
    sources.push(request.goal.as_str());

    if let Some(subject) = first_weather_subject(sources.iter().copied()) {
        return ensure_weather_suffix(subject);
    }

    "天气".to_string()
}

pub fn weather_search_url(request: &AgentRequest) -> String {
    let query = weather_query_text(request);
    let encoded: String = form_urlencoded::byte_serialize(query.as_bytes())
        .collect::<String>()
        .replace('+', "%20");
    format!("https://www.baidu.com/s?wd={encoded}")
}

fn extract_weather_subject(text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }
    capture_city(&CJK_WEATHER_REGEX, text)
        .or_else(|| capture_city(&LATIN_WEATHER_IN_REGEX, text))
        .or_else(|| capture_city(&LATIN_WEATHER_SUFFIX_REGEX, text))
        .map(|subject| trim_subject(&subject))
}

fn capture_city(regex: &Regex, text: &str) -> Option<String> {
    regex
        .captures_iter(text)
        .filter_map(|caps| caps.name("city"))
        .map(|m| m.as_str().trim().to_string())
        .filter(|subject| !subject.is_empty())
        .last()
}

fn ensure_weather_suffix(subject: String) -> String {
    let trimmed = strip_weather_keywords(subject.trim());
    if trimmed.is_empty() {
        return "天气".to_string();
    }
    format!("{trimmed}天气")
}

pub fn first_weather_subject<'a>(sources: impl IntoIterator<Item = &'a str>) -> Option<String> {
    for source in sources {
        if let Some(subject) = extract_weather_subject(source) {
            if subject.chars().count() >= 2 {
                if !is_placeholder_subject(&subject) {
                    return Some(subject);
                }
            }
        }
    }
    None
}

fn is_placeholder_subject(subject: &str) -> bool {
    const PLACEHOLDER_PATTERNS: &[&str] = &[
        "目标城市",
        "指定城市",
        "我的城市",
        "target city",
        "your city",
    ];
    let lower = subject.to_ascii_lowercase();
    PLACEHOLDER_PATTERNS.iter().any(|pattern| {
        let needle = pattern.to_ascii_lowercase();
        lower.contains(&needle)
    })
}

fn trim_subject(text: &str) -> String {
    let mut cleaned = text.trim();
    if cleaned.is_empty() {
        return String::new();
    }
    // Drop surrounding punctuation that frequently wraps prompts.
    cleaned = cleaned.trim_matches(|ch: char| matches!(ch, '"' | '\'' | '“' | '”'));
    cleaned = strip_query_prefix(cleaned);
    cleaned = strip_time_prefix(cleaned);
    cleaned
        .chars()
        .filter(|ch| !ch.is_control())
        .take(32)
        .collect()
}

fn strip_weather_keywords<'a>(text: &'a str) -> &'a str {
    let mut cleaned = text.trim_end();
    loop {
        if let Some(next) = cleaned
            .strip_suffix("天气情况")
            .or_else(|| cleaned.strip_suffix("气温"))
            .or_else(|| cleaned.strip_suffix("天气"))
        {
            cleaned = next.trim_end();
            continue;
        }
        if let Some(next) = strip_ascii_suffix(cleaned, "weather")
            .or_else(|| strip_ascii_suffix(cleaned, "forecast"))
        {
            cleaned = next.trim_end();
            continue;
        }
        break;
    }
    cleaned.trim()
}

fn strip_ascii_suffix<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let text_bytes = text.as_bytes();
    let suffix_bytes = suffix.as_bytes();
    if text_bytes.len() < suffix_bytes.len() {
        return None;
    }
    let start = text_bytes.len() - suffix_bytes.len();
    let mut matches = true;
    for (a, b) in text_bytes[start..].iter().zip(suffix_bytes.iter()) {
        if a.to_ascii_lowercase() != b.to_ascii_lowercase() {
            matches = false;
            break;
        }
    }
    if !matches {
        return None;
    }
    text.get(..start)
}

fn strip_query_prefix<'a>(text: &'a str) -> &'a str {
    const PREFIXES: &[&str] = &[
        "查询一下",
        "查询下",
        "帮忙查询",
        "帮忙查",
        "帮我查询",
        "帮我查",
        "请帮忙查",
        "请帮我查",
        "请查询",
        "查一下",
        "查一查",
        "查询",
        "查",
    ];
    for prefix in PREFIXES {
        if let Some(remainder) = text.strip_prefix(prefix) {
            let trimmed = remainder.trim_start();
            if trimmed.chars().count() >= 2 {
                return trimmed;
            }
        }
    }
    text
}

fn strip_time_prefix<'a>(text: &'a str) -> &'a str {
    const TIME_PREFIXES: &[&str] = &[
        "今天", "明天", "后天", "今晚", "明早", "本周", "下周", "这周", "下", "今", "明", "后",
    ];
    let mut cleaned = text;
    loop {
        let mut matched = false;
        for prefix in TIME_PREFIXES {
            if let Some(remainder) = cleaned.strip_prefix(prefix) {
                let trimmed = remainder.trim_start();
                if trimmed.chars().count() >= 2 {
                    cleaned = trimmed;
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            break;
        }
    }
    cleaned
}

static CJK_WEATHER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?P<city>[\u{4E00}-\u{9FFF}]{2,8})(?:的)?(?:天气|气温)")
        .expect("cjk weather regex")
});

static LATIN_WEATHER_IN_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)weather\s+(?:in|for)\s+(?P<city>[a-zA-Z][a-zA-Z\s-]{1,40})")
        .expect("latin weather in regex")
});

static LATIN_WEATHER_SUFFIX_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?P<city>[a-zA-Z][a-zA-Z\s-]{1,40})\s+(?:weather|forecast)")
        .expect("latin weather suffix regex")
});

#[cfg(test)]
mod tests {
    use super::*;
    use soulbrowser_core_types::TaskId;

    #[test]
    fn detects_city_from_cjk_prompt() {
        let mut request = AgentRequest::new(TaskId::new(), "查询深圳天气");
        request.intent.primary_goal = Some("查询深圳天气".to_string());
        assert_eq!(weather_query_text(&request), "深圳天气");
    }

    #[test]
    fn keeps_generic_weather_when_city_missing() {
        let request = AgentRequest::new(TaskId::new(), "帮我打开百度，查询下今天天气");
        assert_eq!(weather_query_text(&request), "今天天气");
    }

    #[test]
    fn detects_latin_city_variants() {
        let request = AgentRequest::new(TaskId::new(), "weather in San Francisco");
        assert_eq!(weather_query_text(&request), "San Francisco天气");
    }

    #[test]
    fn builds_search_url() {
        let request = AgentRequest::new(TaskId::new(), "查询北京天气");
        let url = weather_search_url(&request);
        assert!(url.contains("wd=%E5%8C%97%E4%BA%AC%E5%A4%A9%E6%B0%94"));
    }

    #[test]
    fn placeholder_subjects_are_ignored() {
        let sources = vec!["查询目标城市的最新天气"];
        assert!(first_weather_subject(sources.iter().copied()).is_none());
    }

    #[test]
    fn removes_time_prefixes() {
        let request = AgentRequest::new(TaskId::new(), "下今天济南天气");
        assert_eq!(weather_query_text(&request), "济南天气");
    }

    #[test]
    fn parses_baidu_weather_widget() {
        let observation = serde_json::json!({
            "title": "济南天气_百度搜索",
            "data": {
                "text_sample": "百度首页设置登录 百度一下 济南天气 济南 14:45更新 5° -4~6°C 晴 70 良"
            }
        });
        let value = crate::parsers::weather::parse_weather(&observation).expect("parse weather");
        assert_eq!(value["city"], "济南");
        assert_eq!(value["temperature_low_c"], -4.0);
        assert_eq!(value["temperature_high_c"], 6.0);
    }
}
