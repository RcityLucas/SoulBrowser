use agent_core::{AgentIntentKind, AgentRequest, RequestedOutput};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

/// Attach lightweight intent hints to the request so downstream planners have
/// consistent metadata regardless of which frontend produced the prompt.
pub fn enrich_request_with_intent(request: &mut AgentRequest, prompt: &str) {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return;
    }
    let intent_kind = classify_intent_kind(trimmed);
    request.intent.intent_kind = intent_kind;
    request.metadata.insert(
        "intent_kind".to_string(),
        Value::String(intent_kind.as_str().to_string()),
    );
    if contains_cjk(trimmed) {
        request.intent.preferred_language = Some("zh-CN".to_string());
        request
            .metadata
            .entry("preferred_language".to_string())
            .or_insert_with(|| Value::String("zh-CN".to_string()));
    }
    apply_configured_intent(request, trimmed);
    if request
        .intent
        .primary_goal
        .as_ref()
        .map(|value| value.trim().is_empty())
        .unwrap_or(true)
    {
        request.intent.primary_goal = Some(trimmed.to_string());
        request.metadata.insert(
            "primary_goal".to_string(),
            Value::String(trimmed.to_string()),
        );
    }
    update_todo_snapshot(request);
}

/// Generate a markdown-friendly todo list that mirrors BrowserUse's `todo.md`.
pub fn build_todo_snapshot(request: &AgentRequest) -> Option<String> {
    let mut bullets = Vec::new();

    let goal = request
        .intent
        .primary_goal
        .as_deref()
        .or_else(|| request.metadata.get("primary_goal").and_then(Value::as_str))
        .unwrap_or_else(|| request.goal.as_str());
    if !goal.trim().is_empty() {
        bullets.push(format!("[ ] 达成目标: {}", goal.trim()));
    }

    if !request.constraints.is_empty() {
        bullets.push(format!("[ ] 遵循限制: {}", request.constraints.join("；")));
    }

    if let Some(lang) = request
        .intent
        .preferred_language
        .as_deref()
        .or_else(|| {
            request
                .metadata
                .get("preferred_language")
                .and_then(Value::as_str)
        })
        .filter(|value| !value.trim().is_empty())
    {
        bullets.push(format!("[ ] 回复语言: {}", lang));
    }

    if bullets.is_empty() {
        None
    } else {
        Some(bullets.join("\n"))
    }
}

/// Persist the todo snapshot inside `request.metadata` so prompts can surface it.
pub fn update_todo_snapshot(request: &mut AgentRequest) {
    if let Some(snapshot) = build_todo_snapshot(request) {
        request
            .metadata
            .insert("todo_snapshot".to_string(), Value::String(snapshot));
    } else {
        request.metadata.remove("todo_snapshot");
    }
}

fn contains_cjk(input: &str) -> bool {
    input
        .chars()
        .any(|ch| matches!(ch, '\u{4E00}'..='\u{9FFF}'))
}

const INFORMATIONAL_HINTS: &[&str] = &[
    "查询", "总结", "对比", "行情", "分析", "research", "compare", "analysis", "weather", "report",
];

fn classify_intent_kind(prompt: &str) -> AgentIntentKind {
    let lower = prompt.to_ascii_lowercase();
    if INFORMATIONAL_HINTS.iter().any(|hint| {
        let trimmed = hint.trim();
        !trimmed.is_empty()
            && (prompt.contains(trimmed) || lower.contains(&trimmed.to_ascii_lowercase()))
    }) {
        AgentIntentKind::Informational
    } else {
        AgentIntentKind::Operational
    }
}

fn apply_configured_intent(request: &mut AgentRequest, prompt: &str) {
    let Some((intent_id, definition)) =
        match_builtin_intent(prompt).or_else(|| match_intent(prompt))
    else {
        return;
    };

    request.intent.intent_id = Some(intent_id.clone());
    request
        .metadata
        .insert("intent_id".to_string(), Value::String(intent_id));

    if let Some(goal) = definition.primary_goal.as_deref() {
        let has_goal = request
            .intent
            .primary_goal
            .as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        if !has_goal {
            request.intent.primary_goal = Some(goal.to_string());
            request
                .metadata
                .insert("primary_goal".to_string(), Value::String(goal.to_string()));
        }
    }

    if request.intent.target_sites.is_empty() && !definition.primary_sites.is_empty() {
        request.intent.target_sites = definition.primary_sites.clone();
        request.intent.target_sites_are_hints = true;
        request.metadata.insert(
            "target_sites".to_string(),
            Value::Array(
                request
                    .intent
                    .target_sites
                    .iter()
                    .map(|site| Value::String(site.clone()))
                    .collect(),
            ),
        );
    }

    if let Some(lang) = definition.preferred_language.as_deref() {
        request.intent.preferred_language = Some(lang.to_string());
        request.metadata.insert(
            "preferred_language".to_string(),
            Value::String(lang.to_string()),
        );
    }

    if let Some(output) = definition.output.as_ref() {
        if let Some(schema) = output.schema.as_deref() {
            let mut requested = RequestedOutput::new(schema);
            requested.description = output.description.clone();
            requested.include_screenshot = output.include_screenshot.unwrap_or(false);
            request.intent.required_outputs = vec![requested];
            request.metadata.insert(
                "required_output_schema".to_string(),
                Value::String(schema.to_string()),
            );
        }
    }

    if !definition.blockers.is_empty() {
        request.intent.blocker_remediations = definition
            .blockers
            .iter()
            .map(|(kind, remediation)| (kind.clone(), remediation.clone()))
            .collect();
    }

    if !definition.validation_keywords.is_empty() {
        request.intent.validation_keywords = definition.validation_keywords.clone();
        let values: Vec<Value> = definition
            .validation_keywords
            .iter()
            .filter(|value| !value.trim().is_empty())
            .map(|value| Value::String(value.to_string()))
            .collect();
        if !values.is_empty() {
            request.metadata.insert(
                "intent_validation_keywords".to_string(),
                Value::Array(values),
            );
        }
    }

    if !definition.allowed_domains.is_empty() {
        request.intent.allowed_domains = definition.allowed_domains.clone();
        let values: Vec<Value> = definition
            .allowed_domains
            .iter()
            .filter(|value| !value.trim().is_empty())
            .map(|value| Value::String(value.to_string()))
            .collect();
        if !values.is_empty() {
            request
                .metadata
                .insert("intent_allowed_domains".to_string(), Value::Array(values));
        }
    }
}

fn match_intent(prompt: &str) -> Option<(String, IntentDefinition)> {
    let config = load_intent_config()?;
    config
        .entries
        .iter()
        .find(|entry| entry.definition.matches(prompt))
        .map(|entry| (entry.id.clone(), entry.definition.clone()))
}

fn match_builtin_intent(prompt: &str) -> Option<(String, IntentDefinition)> {
    if matches_market_quote_prompt(prompt) {
        return Some(("market_quote_lookup".to_string(), market_quote_definition()));
    }
    None
}

fn matches_market_quote_prompt(prompt: &str) -> bool {
    const PRICE_KEYWORDS: &[&str] = &[
        "行情", "价格", "报价", "走势", "price", "quote", "期货", "现货",
    ];
    const METAL_KEYWORDS: &[&str] = &[
        "铜",
        "铝",
        "镍",
        "锌",
        "铅",
        "锡",
        "金属",
        "贵金属",
        "金价",
        "银价",
        "银子",
        "黄金",
        "金子",
        "白银",
        "silver",
        "gold",
        "oil",
        "原油",
        "metal",
        "lme",
        "shfe",
        "伦敦",
        "上海期货",
        "东方财富",
    ];
    let lower = prompt.to_ascii_lowercase();
    let matches_any = |needles: &[&str]| {
        needles.iter().any(|needle| {
            let trimmed = needle.trim();
            if trimmed.is_empty() {
                return false;
            }
            prompt.contains(trimmed) || lower.contains(&trimmed.to_ascii_lowercase())
        })
    };
    matches_any(PRICE_KEYWORDS) && matches_any(METAL_KEYWORDS)
}

fn market_quote_definition() -> IntentDefinition {
    let mut blockers = HashMap::new();
    blockers.insert("captcha".to_string(), "require_manual_captcha".to_string());
    blockers.insert(
        "login_wall".to_string(),
        "require_manual_captcha".to_string(),
    );
    blockers.insert("blank_page".to_string(), "auto_retry".to_string());
    IntentDefinition {
        triggers: Vec::new(),
        primary_goal: Some("查询指定金属的最新行情".to_string()),
        primary_sites: vec![
            "https://quote.eastmoney.com/qh/CU0.html".to_string(),
            "https://data.eastmoney.com/metal/cu.html".to_string(),
        ],
        output: Some(IntentOutput {
            schema: Some("metal_price_v1.json".to_string()),
            include_screenshot: Some(true),
            description: Some(
                "Array of {metal,price,contract,market,currency,as_of,source_url}".to_string(),
            ),
        }),
        preferred_language: Some("zh-CN".to_string()),
        blockers,
        validation_keywords: vec![
            "金属".to_string(),
            "行情".to_string(),
            "期货".to_string(),
            "报价".to_string(),
        ],
        allowed_domains: vec![
            "quote.eastmoney.com".to_string(),
            "data.eastmoney.com".to_string(),
            "eastmoney.com".to_string(),
        ],
    }
}

fn load_intent_config() -> Option<IntentConfig> {
    let path = intent_config_path()?;
    {
        let cache = intent_cache().read().unwrap();
        if cache.path.as_ref() == Some(&path) {
            return cache.config.clone();
        }
    }

    let bytes = fs::read(&path).ok()?;
    let file: IntentConfigFile = serde_yaml::from_slice(&bytes).ok()?;
    let config = IntentConfig::from(file);
    let mut cache = intent_cache().write().unwrap();
    cache.path = Some(path);
    cache.config = Some(config.clone());
    Some(config)
}

fn intent_config_path() -> Option<PathBuf> {
    if let Ok(env_path) = env::var("SOULBROWSER_INTENT_CONFIG") {
        let path = PathBuf::from(env_path);
        if path.exists() {
            return Some(path);
        }
    }
    let default = Path::new("config/intent_config.yaml").to_path_buf();
    if default.exists() {
        Some(default)
    } else {
        let fallback = Path::new("config/defaults/intent_config.yaml").to_path_buf();
        if fallback.exists() {
            Some(fallback)
        } else {
            None
        }
    }
}

fn intent_cache() -> &'static RwLock<IntentConfigCache> {
    static CACHE: OnceLock<RwLock<IntentConfigCache>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(IntentConfigCache::default()))
}

#[cfg(test)]
fn reset_intent_cache_for_tests() {
    if let Ok(mut guard) = intent_cache().write() {
        *guard = IntentConfigCache::default();
    }
}

#[derive(Default)]
struct IntentConfigCache {
    path: Option<PathBuf>,
    config: Option<IntentConfig>,
}

#[derive(Clone)]
struct IntentConfig {
    entries: Vec<IntentEntry>,
}

impl From<IntentConfigFile> for IntentConfig {
    fn from(value: IntentConfigFile) -> Self {
        let entries = value
            .intents
            .into_iter()
            .map(|(id, definition)| IntentEntry { id, definition })
            .collect();
        Self { entries }
    }
}

#[derive(Clone)]
struct IntentEntry {
    id: String,
    definition: IntentDefinition,
}

#[derive(Debug, Clone, Deserialize)]
struct IntentConfigFile {
    intents: HashMap<String, IntentDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
struct IntentDefinition {
    #[serde(default)]
    triggers: Vec<String>,
    #[serde(default)]
    primary_goal: Option<String>,
    #[serde(default, rename = "primary_sites")]
    primary_sites: Vec<String>,
    #[serde(default)]
    output: Option<IntentOutput>,
    #[serde(default)]
    preferred_language: Option<String>,
    #[serde(default)]
    blockers: HashMap<String, String>,
    #[serde(default)]
    validation_keywords: Vec<String>,
    #[serde(default)]
    allowed_domains: Vec<String>,
}

impl IntentDefinition {
    fn matches(&self, prompt: &str) -> bool {
        if self.triggers.is_empty() {
            return false;
        }
        let haystack_lower = prompt.to_ascii_lowercase();
        self.triggers.iter().any(|trigger| {
            let trimmed = trigger.trim();
            if trimmed.is_empty() {
                return false;
            }
            haystack_lower.contains(&trimmed.to_ascii_lowercase()) || prompt.contains(trimmed)
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
struct IntentOutput {
    schema: Option<String>,
    #[serde(default)]
    include_screenshot: Option<bool>,
    #[serde(default)]
    description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{AgentPlanner, AgentToolKind, PlannerConfig, RuleBasedPlanner};
    use soulbrowser_core_types::TaskId;
    use std::env;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn todo_snapshot_includes_goal_and_constraints() {
        let mut request = AgentRequest::new(TaskId::new(), "test goal");
        request.constraints.push("be careful".to_string());
        let snapshot = build_todo_snapshot(&request).expect("snapshot");
        assert!(snapshot.contains("test goal"));
        assert!(snapshot.contains("be careful"));
    }

    #[test]
    fn enrich_request_sets_language_for_cjk() {
        let mut request = AgentRequest::new(TaskId::new(), "");
        enrich_request_with_intent(&mut request, "查看行情");
        assert_eq!(request.intent.preferred_language.as_deref(), Some("zh-CN"));
        let lang_meta = request
            .metadata
            .get("preferred_language")
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(lang_meta, "zh-CN");
    }

    #[test]
    fn builtin_market_quote_intent_detects_keywords() {
        let mut request = AgentRequest::new(TaskId::new(), "我想看最新铜价行情");
        enrich_request_with_intent(&mut request, "我想看最新铜价行情");
        assert_eq!(
            request.intent.intent_id.as_deref(),
            Some("market_quote_lookup")
        );
        assert_eq!(
            request
                .intent
                .required_outputs
                .first()
                .map(|output| output.schema.as_str()),
            Some("metal_price_v1.json")
        );
    }

    #[test]
    fn configured_intent_preserves_user_goal_and_sites() {
        let mut request = AgentRequest::new(TaskId::new(), "请帮我看看白银报价");
        request.intent.primary_goal = Some("我自己的目标".to_string());
        request.intent.target_sites = vec!["https://user.example.com".to_string()];
        request.intent.target_sites_are_hints = false;
        enrich_request_with_intent(&mut request, "请帮我看看白银报价");

        assert_eq!(request.intent.primary_goal.as_deref(), Some("我自己的目标"));
        assert_eq!(
            request.intent.target_sites,
            vec!["https://user.example.com".to_string()]
        );
        assert!(!request.intent.target_sites_are_hints);
    }

    #[test]
    fn applies_predefined_intent_from_config() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("intent.yaml");
        fs::write(
            &path,
            r#"
intents:
  special:
    triggers: ["special run"]
    primary_goal: "Run special workflow"
    primary_sites:
      - "https://example.com"
      - "https://backup.example.com"
    output:
      schema: market_info_v1.json
      include_screenshot: true
      description: "Special report"
    preferred_language: en-US
    blockers:
      captcha: require_manual_captcha
        "#,
        )
        .expect("write config");

        reset_intent_cache_for_tests();
        env::set_var("SOULBROWSER_INTENT_CONFIG", path.to_str().unwrap());

        let mut request = AgentRequest::new(TaskId::new(), "special run");
        enrich_request_with_intent(&mut request, "Please special run this workflow");

        assert_eq!(request.intent.intent_id.as_deref(), Some("special"));
        assert_eq!(
            request.intent.primary_goal.as_deref(),
            Some("Run special workflow")
        );
        assert_eq!(request.intent.target_sites.len(), 2);
        assert!(request.intent.target_sites_are_hints);
        assert_eq!(request.intent.preferred_language.as_deref(), Some("en-US"));
        assert_eq!(request.intent.required_outputs.len(), 1);
        let output = &request.intent.required_outputs[0];
        assert_eq!(output.schema, "market_info_v1.json");
        assert!(output.include_screenshot);
        assert!(request
            .intent
            .blocker_remediations
            .iter()
            .any(
                |(kind, remediation)| kind == "captcha" && remediation == "require_manual_captcha"
            ));

        env::remove_var("SOULBROWSER_INTENT_CONFIG");
        reset_intent_cache_for_tests();
    }

    #[test]
    fn classify_sets_informational_for_queries() {
        let mut request = AgentRequest::new(TaskId::new(), "查询深圳天气");
        enrich_request_with_intent(&mut request, "请查询深圳天气并总结");
        assert_eq!(request.intent.intent_kind, AgentIntentKind::Informational);
        let kind_meta = request
            .metadata
            .get("intent_kind")
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(kind_meta, "informational");
    }

    #[test]
    fn classify_defaults_to_operational() {
        let mut request = AgentRequest::new(TaskId::new(), "登录账户");
        enrich_request_with_intent(&mut request, "请登录账户并修改密码");
        assert_eq!(request.intent.intent_kind, AgentIntentKind::Operational);
    }

    #[test]
    fn market_prompt_generates_validated_plan() {
        reset_intent_cache_for_tests();
        let prompt = "通过东方财富查一下白银走势";
        let mut request = AgentRequest::new(TaskId::new(), prompt);
        enrich_request_with_intent(&mut request, prompt);

        let planner = RuleBasedPlanner::new(PlannerConfig::default());
        let outcome = planner.draft_plan(&request).expect("plan");
        let plan = outcome.plan;

        assert!(contains_tool(&plan, "market.quote.fetch"));
        assert!(contains_tool(&plan, "data.validate-target"));
        assert!(contains_tool(&plan, "data.parse.metal_price"));
        assert!(contains_tool(&plan, "data.deliver.structured"));
    }

    fn contains_tool(plan: &agent_core::AgentPlan, tool_name: &str) -> bool {
        plan.steps.iter().any(|step| match &step.tool.kind {
            AgentToolKind::Custom { name, .. } => name.eq_ignore_ascii_case(tool_name),
            _ => false,
        })
    }
}
