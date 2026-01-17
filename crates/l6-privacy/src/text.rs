use crate::policy::{HashAlg, PiiRule};
use regex::Regex;
use sha2::{Digest, Sha256};

pub fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

pub fn mask_pii(text: &str, rules: &[PiiRule]) -> String {
    let mut masked = normalize(text);
    for rule in rules {
        if let Ok(re) = Regex::new(&rule.pattern) {
            masked = re.replace_all(&masked, "***").into_owned();
        }
    }
    masked
}

pub fn digest(text: &str, alg: HashAlg, max_len: usize) -> (String, usize) {
    let normalized = normalize(text);
    let total_len = normalized.chars().count();
    let truncated: String = normalized.chars().take(max_len).collect();
    let mut hasher = Sha256::new();
    hasher.update(truncated.as_bytes());
    let hash = hex::encode(hasher.finalize());
    match alg {
        HashAlg::Sha256 => (format!("sha256:{}", hash), total_len),
        HashAlg::HmacSha256 => (format!("hmac-sha256:{}", hash), total_len),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::PiiRule;

    #[test]
    fn masks_basic_email() {
        let masked = mask_pii(
            "contact me at foo@example.com",
            &[PiiRule {
                name: "email".into(),
                pattern: r"(?i)[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}".into(),
            }],
        );
        assert!(!masked.contains("foo@example.com"));
    }
}
