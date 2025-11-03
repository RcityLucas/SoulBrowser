pub fn url(raw: &str) -> String {
    match url::Url::parse(raw) {
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
        let mut trimmed = raw[..max_len].to_string();
        trimmed.push('…');
        trimmed
    } else {
        raw.to_string()
    }
}
