use url::Url;

pub fn url(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(parsed) => format!(
            "{}://{}{}",
            parsed.scheme(),
            parsed.host_str().unwrap_or(""),
            parsed.path()
        ),
        Err(_) => raw.to_string(),
    }
}

pub fn title(raw: &str, max_len: usize) -> String {
    if raw.len() > max_len {
        let mut truncated = raw[..max_len].to_string();
        truncated.push_str("...");
        truncated
    } else {
        raw.to_string()
    }
}

pub fn value_hash(_text: &str) -> Option<String> {
    // placeholder: a real implementation would apply a salted hash
    None
}
