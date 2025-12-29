use once_cell::sync::Lazy;

static CAPTCHA_PATTERNS: Lazy<Vec<&'static str>> = Lazy::new(|| {
    vec![
        "verify you're a human",
        "verify you are a human",
        "captcha",
        "enter the characters",
        "prove you're human",
        "请输入验证码",
        "输入验证码",
        "验证码",
    ]
});

static TOO_MANY_REQUESTS_PATTERNS: Lazy<Vec<&'static str>> = Lazy::new(|| {
    vec![
        "访问过于频繁",
        "操作频繁",
        "too many requests",
        "unusual traffic",
    ]
});

/// Detect whether the captured page content looks like an access block (403/404/captcha).
/// Returns a short human-readable reason when a blocker is detected.
pub fn detect_block_reason(title: &str, body: &str, url: Option<&str>) -> Option<String> {
    let title_lower = title.to_ascii_lowercase();
    let body_lower = body.to_ascii_lowercase();
    let url_lower = url.unwrap_or("").to_ascii_lowercase();

    if url_lower.contains("wappass.baidu.com") || url_lower.contains("verify.baidu.com") {
        return Some("Baidu returned a verification page".to_string());
    }

    if title_lower.contains("403") && title_lower.contains("forbidden")
        || body_lower.contains("403 forbidden")
    {
        return Some("Page reports 403 Forbidden".to_string());
    }
    if title_lower.contains("404") && title_lower.contains("not found")
        || body_lower.contains("404 not found")
    {
        return Some("Page reports 404 Not Found".to_string());
    }
    if title_lower.contains("access denied") || body_lower.contains("access denied") {
        return Some("Access denied notice detected".to_string());
    }

    if CAPTCHA_PATTERNS
        .iter()
        .any(|pattern| body_lower.contains(pattern) || title_lower.contains(pattern))
    {
        return Some("Page requests captcha verification".to_string());
    }

    if TOO_MANY_REQUESTS_PATTERNS
        .iter()
        .any(|pattern| body_lower.contains(pattern) || title_lower.contains(pattern))
    {
        return Some("Site reports too many requests".to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::detect_block_reason;

    #[test]
    fn detects_403() {
        let reason = detect_block_reason("403 Forbidden", "", None);
        assert_eq!(reason.unwrap(), "Page reports 403 Forbidden");
    }

    #[test]
    fn detects_captcha_keywords() {
        let reason = detect_block_reason("", "verify you're a human before continuing", None);
        assert_eq!(reason.unwrap(), "Page requests captcha verification");
    }

    #[test]
    fn detects_baidu_redirect() {
        let reason = detect_block_reason(
            "百度安全验证",
            "",
            Some("https://wappass.baidu.com/wappass"),
        );
        assert_eq!(reason.unwrap(), "Baidu returned a verification page");
    }
}
