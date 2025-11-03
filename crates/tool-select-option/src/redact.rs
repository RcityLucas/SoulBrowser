use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

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
        let mut trimmed = raw[..max_len].to_string();
        trimmed.push('…');
        trimmed
    } else {
        raw.to_string()
    }
}

pub fn selection_hash(values: &[String]) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    values.hash(&mut hasher);
    Some(format!("{:016x}", hasher.finish()))
}
