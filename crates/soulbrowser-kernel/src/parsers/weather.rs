use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::parsers::{
    extract_observation_metadata, normalize_whitespace, text_from_candidates, ObservationMetadata,
};

const SCHEMA_ID: &str = "weather_report_v1";

#[derive(Debug, Serialize)]
pub struct WeatherReportOutput {
    pub schema: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_high_c: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_low_c: Option<f64>,
}

/// Parse weather widgets or snippets from a generic observation snapshot.
pub fn parse_weather(observation: &Value) -> Result<Value> {
    let metadata = extract_observation_metadata(observation);
    if let Some(url) = metadata.source_url.as_deref() {
        if looks_like_baidu_home(url) {
            bail!(
                "observation captured Baidu home page ({}); weather results never loaded",
                url
            );
        }
    }
    let text = gather_observation_text(observation);
    if text.trim().is_empty() {
        bail!("observation missing text for weather parsing");
    }

    let mut city = infer_city(&metadata, &text);
    let (mut low_c, mut high_c) = infer_temperatures(&text);
    let mut condition = infer_condition(&text);

    let mobile_source = metadata
        .source_url
        .as_deref()
        .map(|url| url.contains("m.weather.com.cn"))
        .unwrap_or(false);

    if mobile_source || city.is_none() || low_c.is_none() || high_c.is_none() || condition.is_none()
    {
        if let Some((fallback_city, fallback_condition, fallback_high, fallback_low)) =
            parse_mobile_forecast(&text)
        {
            city = Some(fallback_city);
            condition = Some(fallback_condition);
            high_c = Some(fallback_high);
            low_c = Some(fallback_low);
        }
    }

    if let (Some(low_value), Some(high_value)) = (low_c, high_c) {
        if high_value < low_value {
            low_c = Some(high_value);
            high_c = Some(low_value);
        }
    }

    if city.is_none() {
        bail!("unable to determine city name from observation");
    }
    if high_c.is_none() || low_c.is_none() {
        bail!("unable to extract temperature high/low from observation");
    }
    if condition.is_none() {
        bail!("unable to extract weather condition from observation");
    }

    let output = WeatherReportOutput {
        schema: SCHEMA_ID,
        source_url: metadata.source_url,
        captured_at: metadata.captured_at,
        city,
        condition,
        temperature_high_c: high_c,
        temperature_low_c: low_c,
    };

    serde_json::to_value(output).context("serialize weather report output")
}

fn gather_observation_text(observation: &Value) -> String {
    let mut chunks = Vec::new();

    if let Some(sample) = text_from_candidates(observation, &["text_sample", "data.text_sample"]) {
        chunks.push(sample);
    }

    if let Some(description) =
        text_from_candidates(observation, &["description", "data.description"])
    {
        chunks.push(description);
    }

    for heading in collect_entries(observation, "headings").into_iter().take(6) {
        if let Some(text) = heading.get("text").and_then(Value::as_str) {
            chunks.push(text.to_string());
        }
    }

    for paragraph in collect_entries(observation, "paragraphs")
        .into_iter()
        .take(4)
    {
        if let Some(text) = paragraph.as_str() {
            chunks.push(text.to_string());
        }
    }

    if let Some(hero) = text_from_candidates(
        observation,
        &["hero_text", "identity", "data.hero_text", "data.identity"],
    ) {
        chunks.push(hero);
    }

    for entry in collect_entries(observation, "key_values")
        .into_iter()
        .take(10)
    {
        let label = entry
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let value = entry
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if label.is_empty() && value.is_empty() {
            continue;
        }
        chunks.push(format!("{} {}", label, value).trim().to_string());
    }

    if let Some(title) = text_from_candidates(observation, &["title", "data.title"]) {
        chunks.push(title);
    }

    normalize_whitespace(&chunks.join(" "))
}

fn collect_entries<'a>(observation: &'a Value, key: &str) -> Vec<&'a Value> {
    let mut entries = Vec::new();

    if let Some(array) = observation.get(key).and_then(Value::as_array) {
        entries.extend(array.iter());
    }

    if let Some(array) = observation
        .get("data")
        .and_then(|data| data.get(key))
        .and_then(Value::as_array)
    {
        entries.extend(array.iter());
    }

    entries
}

fn infer_city(metadata: &ObservationMetadata, text: &str) -> Option<String> {
    let candidates = [
        metadata.primary_title(),
        metadata.hero_text.clone(),
        Some(text.to_string()),
    ];

    for candidate in candidates.into_iter().flatten() {
        if let Some(city) = extract_city(&candidate) {
            return Some(city);
        }
    }

    if let Some(city) = extract_city_from_leading_text(text) {
        return Some(city);
    }

    if let Some(city) = extract_city_from_url(metadata) {
        return Some(city);
    }

    None
}

fn extract_city(input: &str) -> Option<String> {
    for regex in [&*CITY_SUFFIX_RE, &*CITY_PREFIX_RE, &*CITY_CHINESE_SUFFIX_RE] {
        if let Some(caps) = regex.captures(input) {
            if let Some(city_match) = caps.name("city") {
                let city = normalize_whitespace(city_match.as_str());
                if !city.is_empty() {
                    return Some(city);
                }
            }
        }
    }
    None
}

fn extract_city_from_leading_text(text: &str) -> Option<String> {
    if let Some(caps) = CITY_LEADING_DATE_RE.captures(text) {
        if let Some(m) = caps.name("city") {
            let city = normalize_whitespace(m.as_str());
            if !city.is_empty() {
                return Some(city);
            }
        }
    }
    None
}

fn extract_city_from_url(metadata: &ObservationMetadata) -> Option<String> {
    let url = metadata.source_url.as_deref()?;
    if let Ok(parsed) = Url::parse(url) {
        if let Some(domain) = parsed.domain() {
            if domain.contains("weather.com.cn") {
                if let Some(city) = metadata.primary_title() {
                    if let Some(extracted) = extract_city(&city) {
                        return Some(extracted);
                    }
                }
            }
        }
    }
    None
}

fn parse_mobile_forecast(text: &str) -> Option<(String, String, f64, f64)> {
    let caps = CN_MOBILE_FORECAST_RE.captures(text)?;
    let city = caps.name("city")?.as_str().trim().to_string();
    let condition = caps.name("cond")?.as_str().trim().to_string();
    let high = caps.name("high")?.as_str().parse::<f64>().ok()?;
    let low = caps.name("low")?.as_str().parse::<f64>().ok()?;
    Some((city, condition, high, low))
}

fn infer_temperatures(text: &str) -> (Option<f64>, Option<f64>) {
    if let Some(caps) = RANGE_RE.captures(text) {
        let first = caps
            .name("first")
            .and_then(|m| parse_temp_value(m.as_str()));
        let second = caps
            .name("second")
            .and_then(|m| parse_temp_value(m.as_str()));
        if let (Some(a), Some(b)) = (first, second) {
            return (Some(a.min(b)), Some(a.max(b)));
        }
    }

    if let Some(caps) = SLASH_TEMP_RE.captures(text) {
        let first = caps
            .name("first")
            .and_then(|m| parse_temp_value(m.as_str()));
        let second = caps
            .name("second")
            .and_then(|m| parse_temp_value(m.as_str()));
        if let (Some(a), Some(b)) = (first, second) {
            return (Some(a.min(b)), Some(a.max(b)));
        }
    }

    let mut low = None;
    let mut high = None;

    if let Some(caps) = HIGH_LABEL_RE.captures(text) {
        high = caps
            .name("value")
            .and_then(|m| parse_temp_value(m.as_str()));
    }
    if let Some(caps) = LOW_LABEL_RE.captures(text) {
        low = caps
            .name("value")
            .and_then(|m| parse_temp_value(m.as_str()));
    }

    if low.is_some() && high.is_some() {
        return (low, high);
    }

    let mut values: Vec<f64> = TEMP_RE
        .captures_iter(text)
        .filter_map(|caps| caps.name("value"))
        .filter_map(|m| parse_temp_value(m.as_str()))
        .collect();
    values.retain(|value| value.abs() <= 90.0);
    if values.len() >= 2 {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        return (values.first().copied(), values.last().copied());
    }

    (low, high)
}

fn parse_temp_value(raw: &str) -> Option<f64> {
    raw.trim().parse::<f64>().ok()
}

fn infer_condition(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for pattern in CONDITION_KEYWORDS {
        if pattern.is_ascii {
            if lower.contains(pattern.needle) {
                return Some(pattern.label.to_string());
            }
        } else if text.contains(pattern.needle) {
            return Some(pattern.label.to_string());
        }
    }
    None
}

fn looks_like_baidu_home(url: &str) -> bool {
    if let Ok(parsed) = Url::parse(url) {
        if let Some(domain) = parsed.domain() {
            if domain.eq_ignore_ascii_case("www.baidu.com") {
                let path = parsed.path().trim_matches('/');
                return path.is_empty();
            }
        }
    }
    false
}

struct ConditionPattern {
    needle: &'static str,
    label: &'static str,
    is_ascii: bool,
}

static CONDITION_KEYWORDS: &[ConditionPattern] = &[
    ConditionPattern {
        needle: "晴",
        label: "晴",
        is_ascii: false,
    },
    ConditionPattern {
        needle: "多云",
        label: "多云",
        is_ascii: false,
    },
    ConditionPattern {
        needle: "阴",
        label: "阴",
        is_ascii: false,
    },
    ConditionPattern {
        needle: "雨",
        label: "雨",
        is_ascii: false,
    },
    ConditionPattern {
        needle: "雪",
        label: "雪",
        is_ascii: false,
    },
    ConditionPattern {
        needle: "雷阵雨",
        label: "雷阵雨",
        is_ascii: false,
    },
    ConditionPattern {
        needle: "sunny",
        label: "Sunny",
        is_ascii: true,
    },
    ConditionPattern {
        needle: "cloudy",
        label: "Cloudy",
        is_ascii: true,
    },
    ConditionPattern {
        needle: "overcast",
        label: "Overcast",
        is_ascii: true,
    },
    ConditionPattern {
        needle: "clear",
        label: "Clear",
        is_ascii: true,
    },
    ConditionPattern {
        needle: "showers",
        label: "Showers",
        is_ascii: true,
    },
    ConditionPattern {
        needle: "rain",
        label: "Rain",
        is_ascii: true,
    },
    ConditionPattern {
        needle: "snow",
        label: "Snow",
        is_ascii: true,
    },
    ConditionPattern {
        needle: "storm",
        label: "Storm",
        is_ascii: true,
    },
    ConditionPattern {
        needle: "thunder",
        label: "Thunderstorm",
        is_ascii: true,
    },
];

static CITY_SUFFIX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?P<city>[\p{L}\p{Han} ]{2,24})\s*(?:天气|weather)(?:[^\p{L}\p{Han}]|$)")
        .expect("city suffix regex")
});
static CITY_PREFIX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)weather\s+(?:in|for)?\s*(?P<city>[\p{L} ]{2,24})").expect("city prefix regex")
});
static CITY_CHINESE_SUFFIX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?P<city>[\p{Han}]{2,8})(?:天气|未来|气温)").expect("chinese city regex")
});
static CITY_LEADING_DATE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<city>[\p{Han}A-Za-z]{2,20})\s+\d{4}[/年]")
        .expect("city leading date regex")
});
static RANGE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?P<first>-?\d{1,2})\s*(?:°|度)?\s*(?:c|C)?\s*(?:/|[–—~～到至])\s*(?P<second>-?\d{1,2})",
    )
    .expect("temperature range regex")
});
static SLASH_TEMP_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?P<first>-?\d+(?:\.\d+)?)\s*/\s*(?P<second>-?\d+(?:\.\d+)?)")
        .expect("slash temperature regex")
});
static TEMP_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?P<value>-?\d+(?:\.\d+)?)\s*(?:°|度)?\s*(?:c|C|摄氏度|℃)?")
        .expect("temperature regex")
});
static HIGH_LABEL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:高温|最高|high|highs?)\D{0,3}(?P<value>-?\d+(?:\.\d+)?)")
        .expect("high temperature regex")
});
static LOW_LABEL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:低温|最低|low|lows?)\D{0,3}(?P<value>-?\d+(?:\.\d+)?)")
        .expect("low temperature regex")
});

static CN_MOBILE_FORECAST_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?s)(?P<city>[\p{Han}A-Za-z]{2,20})\s+20\d{2}/\d{2}/\d{2}.*?今天\s+\d{1,2}/\d{1,2}\s+(?P<cond>[\p{Han}A-Za-z]{2,12})\s+(?P<high>-?\d+)\s*/\s*(?P<low>-?\d+)℃",
    )
    .expect("china mobile forecast regex")
});

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_chinese_weather_widget() {
        let observation = json!({
            "title": "北京天气 - 搜索结果",
            "data": {
                "url": "https://weather.example.com/beijing",
                "fetched_at": "2025-02-01T00:00:00Z",
                "text_sample": "北京天气 晴 0° / 6°C 未来几天持续多云",
                "headings": [{"text": "北京天气预报"}],
                "paragraphs": ["白天晴 6°C，夜间 0°C"]
            }
        });

        let output = parse_weather(&observation).expect("parse weather");
        assert_eq!(output["schema"], SCHEMA_ID);
        assert_eq!(output["city"], "北京");
        assert_eq!(output["condition"], "晴");
        assert_eq!(output["temperature_high_c"], 6.0);
        assert_eq!(output["temperature_low_c"], 0.0);
    }

    #[test]
    fn parses_english_range() {
        let observation = json!({
            "title": "Weather in Seattle",
            "data": {
                "url": "https://weather.example.com/seattle",
                "fetched_at": "2025-02-01T00:00:00Z",
                "text_sample": "Weather in Seattle: Cloudy High 52° Low 44°"
            }
        });

        let output = parse_weather(&observation).expect("parse weather");
        assert_eq!(output["city"], "Seattle");
        assert_eq!(output["condition"], "Cloudy");
        assert_eq!(output["temperature_high_c"], 52.0);
        assert_eq!(output["temperature_low_c"], 44.0);
    }

    #[test]
    fn fails_without_temperatures() {
        let observation = json!({
            "title": "上海天气",
            "data": {
                "text_sample": "上海天气 阴",
            }
        });

        assert!(parse_weather(&observation).is_err());
    }

    #[test]
    fn rejects_baidu_homepage_observation() {
        let observation = json!({
            "title": "百度一下，你就知道",
            "data": {
                "url": "https://www.baidu.com/",
                "text_sample": "百度首页",
            }
        });

        let error = parse_weather(&observation).expect_err("should fail for homepage");
        assert!(error.to_string().contains("weather results never loaded"));
    }

    #[test]
    fn parses_city_when_title_has_suffix_punctuation() {
        let observation = json!({
            "title": "济南天气_百度搜索",
            "data": {
                "url": "https://www.baidu.com/s?wd=%E6%B5%8E%E5%8D%97%E5%A4%A9%E6%B0%94",
                "text_sample": "济南 14:45更新 5° -4~6°C 晴",
            }
        });

        let output = parse_weather(&observation).expect("parse weather");
        assert_eq!(output["city"], "济南");
        assert_eq!(output["temperature_low_c"], -4.0);
        assert_eq!(output["temperature_high_c"], 6.0);
    }

    #[test]
    fn parses_mobile_china_weather_forecast() {
        let observation = json!({
            "title": "【济南天气预报15天_济南天气预报15天查询】-中国天气网",
            "data": {
                "url": "https://m.weather.com.cn/mweather15d/101120101.shtml",
                "text_sample": "济南 2025/12/27 ~ 2026/01/10 今天 12/27 多云转晴 8/1℃ 周日 12/28 晴 11/0℃",
            }
        });

        let output = parse_weather(&observation).expect("parse mobile weather");
        assert_eq!(output["city"], "济南");
        assert_eq!(output["condition"], "多云转晴");
        assert_eq!(output["temperature_high_c"], 8.0);
        assert_eq!(output["temperature_low_c"], 1.0);
    }
}
