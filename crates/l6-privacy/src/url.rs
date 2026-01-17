use url::Url;

pub fn redact_url(raw: &str, allow_keys: &[String]) -> String {
    if let Ok(parsed) = Url::parse(raw) {
        let mut redacted = format!("{}{}", parsed.scheme(), "://");
        if let Some(host) = parsed.host_str() {
            redacted.push_str(host);
        }
        if let Some(path) = parsed.path().strip_prefix('/') {
            if !path.is_empty() {
                redacted.push('/');
                redacted.push_str(path);
            }
        }

        let mut filtered = vec![];
        for (key, value) in parsed.query_pairs() {
            if allow_keys.iter().any(|k| k == key.as_ref()) {
                filtered.push(format!("{}={}", key, value));
            } else {
                filtered.push(format!("{}=***", key));
            }
        }
        if !filtered.is_empty() {
            redacted.push('?');
            redacted.push_str(&filtered.join("&"));
        }
        redacted
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_query() {
        let out = redact_url(
            "https://example.com/search?q=hello&safe=1",
            &["safe".into()],
        );
        assert_eq!(out, "https://example.com/search?q=***&safe=1");
    }
}
