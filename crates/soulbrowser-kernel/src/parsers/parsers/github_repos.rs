use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::parsers::{extract_observation_metadata, normalize_whitespace, now_timestamp};

const SCHEMA_ID: &str = "github_repos_v1";

#[derive(Debug, Serialize)]
struct RepoItem {
    name: String,
    description: Option<String>,
    language: Option<String>,
    topics: Vec<String>,
    stars: u64,
    forks: u64,
    watchers: u64,
    open_issues: u64,
    visibility: String,
    default_branch: String,
    html_url: String,
    homepage: Option<String>,
    archived: bool,
    disabled: bool,
    is_fork: bool,
    license: Option<String>,
    pushed_at: Option<String>,
    updated_at: Option<String>,
    created_at: Option<String>,
    owner: Option<String>,
    language_color: Option<String>,
}

#[derive(Debug, Serialize)]
struct RepoOutput {
    schema: &'static str,
    source_username: String,
    fetched_at: String,
    items: Vec<RepoItem>,
}

pub fn parse_github_repos(observation: &Value, username: &str) -> Result<Value> {
    let trimmed_user = username.trim().trim_start_matches('@');
    if trimmed_user.is_empty() {
        bail!("github parser requires non-empty username payload");
    }

    let data = observation.get("data").unwrap_or(observation);
    let links = data
        .get("links")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut seen = std::collections::BTreeMap::new();
    for link in links {
        let url = link.get("url").and_then(Value::as_str).unwrap_or("");
        if url.is_empty() {
            continue;
        }
        if let Some((repo_name, repo_url)) = repo_from_url(url, trimmed_user) {
            let label = link
                .get("text")
                .and_then(Value::as_str)
                .map(normalize_whitespace)
                .filter(|value| !value.is_empty());
            seen.entry(repo_name).or_insert((repo_url, label));
        }
    }

    if seen.is_empty() {
        bail!("unable to locate repository links for user '{}'; navigate to the GitHub profile page first", trimmed_user);
    }

    let metadata = extract_observation_metadata(data);
    let fetched_at = metadata.captured_at.unwrap_or_else(|| now_timestamp());

    let items: Vec<RepoItem> = seen
        .into_iter()
        .map(|(name, (url, label))| RepoItem {
            name,
            description: label,
            language: None,
            topics: Vec::new(),
            stars: 0,
            forks: 0,
            watchers: 0,
            open_issues: 0,
            visibility: "public".to_string(),
            default_branch: "main".to_string(),
            html_url: url,
            homepage: None,
            archived: false,
            disabled: false,
            is_fork: false,
            license: None,
            pushed_at: None,
            updated_at: None,
            created_at: None,
            owner: Some(trimmed_user.to_string()),
            language_color: None,
        })
        .collect();

    let output = RepoOutput {
        schema: SCHEMA_ID,
        source_username: trimmed_user.to_string(),
        fetched_at,
        items,
    };

    serde_json::to_value(output).map_err(|err| anyhow!("serialize github output: {}", err))
}

fn repo_from_url(url: &str, username: &str) -> Option<(String, String)> {
    if url.trim().is_empty() {
        return None;
    }
    let normalized = if url.starts_with("http") {
        url.to_string()
    } else {
        format!("https://github.com{}", url)
    };

    let parsed = Url::parse(&normalized).ok()?;
    let mut segments = parsed.path_segments()?;
    let owner = segments.next()?;
    if owner.eq_ignore_ascii_case(username) {
        let repo = segments.next()?;
        let clean_repo = repo.trim_matches('/');
        if clean_repo.is_empty() {
            return None;
        }
        let repo_url = format!("https://github.com/{}/{}", owner, clean_repo);
        Some((clean_repo.to_string(), repo_url))
    } else {
        None
    }
}
