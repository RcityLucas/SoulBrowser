use std::collections::HashSet;
use url::Url;

use crate::model::AgentRequest;

pub fn derive_guardrail_keywords(request: &AgentRequest) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut keywords = Vec::new();

    for value in request.intent.validation_keywords.iter() {
        push_keyword(&mut keywords, &mut seen, value);
    }
    for value in request
        .intent
        .allowed_domains
        .iter()
        .chain(request.intent.target_sites.iter())
    {
        for token in domain_keyword_tokens(value) {
            push_keyword(&mut keywords, &mut seen, &token);
        }
    }
    if keywords.is_empty() {
        if let Some(goal) = request.intent.primary_goal.as_deref() {
            for term in split_terms(goal) {
                push_keyword(&mut keywords, &mut seen, &term);
            }
        }
    }
    if keywords.is_empty() {
        for term in split_terms(&request.goal) {
            push_keyword(&mut keywords, &mut seen, &term);
        }
    }
    keywords.truncate(8);
    keywords
}

pub fn derive_guardrail_domains(request: &AgentRequest) -> Vec<String> {
    let mut domains = HashSet::new();
    for value in request.intent.allowed_domains.iter() {
        if let Some(domain) = normalize_domain(value) {
            domains.insert(domain.trim_start_matches("www.").to_string());
        }
    }
    for value in request.intent.target_sites.iter() {
        if let Some(domain) = normalize_domain(value) {
            domains.insert(domain.trim_start_matches("www.").to_string());
        }
    }
    if domains.is_empty() {
        let keywords = derive_guardrail_keywords(request);
        for alias in infer_domains_from_keywords(&keywords) {
            domains.insert(alias);
        }
    }
    domains.into_iter().take(6).collect()
}

fn normalize_domain(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains("://") {
        Url::parse(trimmed)
            .ok()
            .and_then(|parsed| parsed.domain().map(|domain| domain.to_ascii_lowercase()))
    } else if trimmed.contains('.') {
        Some(trimmed.trim_start_matches("www.").to_ascii_lowercase())
    } else {
        None
    }
}

fn split_terms(text: &str) -> Vec<String> {
    text.split(|ch: char| {
        matches!(
            ch,
            ',' | '，'
                | '。'
                | ';'
                | '；'
                | ':'
                | '：'
                | '/'
                | '|'
                | '\\'
                | '、'
                | '!'
                | '！'
                | '?'
                | '？'
                | '\t'
                | '\n'
        )
    })
    .map(|chunk| chunk.trim().to_string())
    .filter(|chunk| !chunk.is_empty())
    .collect()
}

fn domain_keyword_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let Some(domain) = normalize_domain(value) else {
        return tokens;
    };
    tokens.push(domain.clone());
    if let Some(first_label) = domain.split('.').next() {
        tokens.push(first_label.to_string());
    }
    let parts: Vec<&str> = domain.split('.').collect();
    if parts.len() >= 2 {
        tokens.push(parts[parts.len() - 2].to_string());
    }
    tokens
}

fn push_keyword(keywords: &mut Vec<String>, seen: &mut HashSet<String>, candidate: &str) {
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return;
    }
    if seen.insert(trimmed.to_string()) {
        keywords.push(trimmed.to_string());
    }
}

fn infer_domains_from_keywords(keywords: &[String]) -> Vec<String> {
    const DOMAIN_ALIASES: &[(&str, &str)] = &[
        ("同花顺", "10jqka.com.cn"),
        ("东方财富", "eastmoney.com"),
        ("新浪财经", "finance.sina.com.cn"),
        ("财联社", "cls.cn"),
        ("金十", "jin10.com"),
        ("上交所", "sse.com.cn"),
        ("深交所", "szse.cn"),
        ("伦敦金属交易所", "lme.com"),
    ];
    let mut inferred = Vec::new();
    for keyword in keywords {
        for (alias, domain) in DOMAIN_ALIASES {
            if keyword.contains(alias) {
                inferred.push(domain.to_string());
            }
        }
    }
    inferred
}

#[cfg(test)]
mod tests {
    use super::*;
    use soulbrowser_core_types::TaskId;

    #[test]
    fn derives_domain_keywords_from_allowed_domains() {
        let mut request = AgentRequest::new(TaskId::new(), "demo");
        request.intent.allowed_domains = vec!["https://quote.eastmoney.com".to_string()];
        let keywords = derive_guardrail_keywords(&request);
        assert!(keywords.iter().any(|kw| kw.contains("quote.eastmoney.com")));
        assert!(keywords.iter().any(|kw| kw.eq("quote")));
    }

    #[test]
    fn infers_guardrail_domains_from_keywords() {
        let mut request = AgentRequest::new(TaskId::new(), "通过同花顺了解镍价");
        request.intent.validation_keywords = vec!["通过同花顺帮我查一下今天镍价".to_string()];
        let domains = derive_guardrail_domains(&request);
        assert!(domains
            .iter()
            .any(|domain| domain.contains("10jqka.com.cn")));
    }
}
