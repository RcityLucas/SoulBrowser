pub fn extract_json_object(raw: &str) -> Option<String> {
    if raw.trim_start().starts_with('{') {
        return Some(trim_symmetric(raw));
    }

    let fence = "```";
    if let Some(start) = raw.find(fence) {
        let after_fence = &raw[start + fence.len()..];
        let after_lang = after_fence.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_');
        if let Some(end) = after_lang.find(fence) {
            let block = &after_lang[..end];
            if block.contains('{') {
                return Some(trim_symmetric(block));
            }
        }
    }

    raw.split('{').skip(1).next().and_then(|rest| {
        let mut depth = 1i32;
        for (idx, ch) in rest.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let mut candidate = String::from("{");
                        candidate.push_str(&rest[..=idx]);
                        return Some(trim_symmetric(&candidate));
                    }
                }
                _ => {}
            }
        }
        None
    })
}

fn trim_symmetric(value: &str) -> String {
    value.trim().trim_matches('`').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_from_fenced_block() {
        let input = "Here is a plan:\n```json\n{\"title\":\"Do it\"}\n```";
        let extracted = extract_json_object(input).expect("json");
        assert!(extracted.contains("\"title\""));
        assert!(extracted.starts_with('{'));
    }

    #[test]
    fn extracts_from_inline_object() {
        let input = "text { \"foo\": 1 } more";
        let extracted = extract_json_object(input).expect("json");
        assert_eq!(extracted, "{ \"foo\": 1 }");
    }

    #[test]
    fn returns_none_when_missing() {
        assert!(extract_json_object("no braces").is_none());
    }
}
