use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use agent_core::{AgentIntentMetadata, AgentRequest, RequestedOutput};
use once_cell::sync::OnceCell;
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde::Deserializer;
use serde_json::Value;
use serde_yaml::Value as YamlValue;
use tracing::warn;

const DEFAULT_INTENT_PATH: &str = "config/intent_config.yaml";

static INTENT_CONFIG: OnceCell<IntentConfig> = OnceCell::new();

pub fn intent_config() -> &'static IntentConfig {
    INTENT_CONFIG.get_or_init(|| {
        let path = resolve_intent_config_path();
        IntentConfig::from_path(path.as_deref()).unwrap_or_else(|| {
            warn!(
                "intent config not found at {:?}; falling back to defaults",
                path
            );
            IntentConfig::default()
        })
    })
}

pub fn enrich_request_with_intent(request: &mut AgentRequest, prompt: &str) {
    if request.intent.intent_id.is_some() {
        return;
    }
    let trimmed_prompt = prompt.trim();
    if trimmed_prompt.is_empty() {
        return;
    }
    if let Some(intent) = intent_config().detect(trimmed_prompt) {
        request.intent = intent;
    } else if contains_cjk(trimmed_prompt) {
        request
            .intent
            .preferred_language
            .get_or_insert_with(|| "zh-CN".to_string());
    }

    if request.intent.primary_goal.is_none() && !trimmed_prompt.is_empty() {
        request.intent.primary_goal = Some(trimmed_prompt.to_string());
    }

    request.sync_intent_metadata();
    update_todo_snapshot(request);
}

fn resolve_intent_config_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("SOULBROWSER_INTENT_CONFIG") {
        let buf = PathBuf::from(path);
        if buf.exists() {
            return Some(buf);
        }
    }
    let candidate = PathBuf::from(DEFAULT_INTENT_PATH);
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
struct IntentDefinition {
    id: String,
    triggers: Vec<String>,
    primary_goal: Option<String>,
    target_sites: Vec<String>,
    required_outputs: Vec<IntentOutput>,
    preferred_language: Option<String>,
    blockers: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct IntentDefinitionSpec {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    triggers: Vec<String>,
    #[serde(default)]
    primary_goal: Option<String>,
    #[serde(default, alias = "primary_sites")]
    target_sites: Vec<String>,
    #[serde(default, alias = "outputs", alias = "output")]
    required_outputs: IntentOutputList,
    #[serde(default)]
    preferred_language: Option<String>,
    #[serde(default)]
    blockers: HashMap<String, String>,
}

impl IntentDefinitionSpec {
    fn into_definition(self, fallback_id: Option<&str>) -> Option<IntentDefinition> {
        let id = self
            .id
            .or_else(|| fallback_id.map(|value| value.to_string()))?;
        Some(IntentDefinition {
            id,
            triggers: self.triggers,
            primary_goal: self.primary_goal,
            target_sites: self.target_sites,
            required_outputs: self.required_outputs.into_vec(),
            preferred_language: self.preferred_language,
            blockers: self.blockers,
        })
    }
}

impl IntentDefinition {
    fn matches(&self, prompt: &str) -> bool {
        if self.triggers.is_empty() {
            return false;
        }
        let haystack = prompt.to_ascii_lowercase();
        self.triggers.iter().any(|trigger| {
            let needle = trigger.to_ascii_lowercase();
            haystack.contains(&needle)
        })
    }

    fn to_metadata(&self, prompt: &str) -> AgentIntentMetadata {
        let mut metadata = AgentIntentMetadata::default();
        metadata.intent_id = Some(self.id.clone());
        metadata.primary_goal = self
            .primary_goal
            .clone()
            .or_else(|| Some(prompt.to_string()));
        if !self.target_sites.is_empty() {
            metadata.target_sites = self.target_sites.clone();
        }
        if !self.required_outputs.is_empty() {
            metadata.required_outputs = self
                .required_outputs
                .iter()
                .map(|req| {
                    let mut output = RequestedOutput::new(&req.schema);
                    output.include_screenshot = req.include_screenshot;
                    output.description = req.description.clone();
                    output
                })
                .collect();
        }
        metadata.preferred_language = self.preferred_language.clone().or_else(|| {
            if contains_cjk(prompt) {
                Some("zh-CN".to_string())
            } else {
                None
            }
        });
        metadata.blocker_remediations = self.blockers.clone();
        metadata
    }
}

/// Generate a markdown-friendly todo snapshot that mirrors BrowserUse's `todo.md`.
pub fn build_todo_snapshot(request: &AgentRequest) -> Option<String> {
    let mut bullets = Vec::new();

    let goal = request
        .intent
        .primary_goal
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| request.goal.as_str());
    if !goal.trim().is_empty() {
        bullets.push(format!("[ ] 达成目标: {}", goal.trim()));
    }

    if !request.intent.target_sites.is_empty() {
        for (idx, site) in request.intent.target_sites.iter().enumerate() {
            bullets.push(format!("[ ] 第{}优先站点: {}", idx + 1, site));
        }
    }

    if !request.intent.required_outputs.is_empty() {
        for output in &request.intent.required_outputs {
            let mut parts = vec![output.schema.clone()];
            if let Some(desc) = output.description.as_deref() {
                if !desc.trim().is_empty() {
                    parts.push(desc.trim().to_string());
                }
            }
            if output.include_screenshot {
                parts.push("需要截图".to_string());
            }
            bullets.push(format!("[ ] 产出结构化结果: {}", parts.join(" | ")));
        }
    }

    if !request.constraints.is_empty() {
        bullets.push(format!("[ ] 遵循限制: {}", request.constraints.join("；")));
    }

    if let Some(lang) = request
        .intent
        .preferred_language
        .as_deref()
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

#[derive(Debug, Clone, Deserialize)]
struct IntentOutput {
    schema: String,
    #[serde(default)]
    include_screenshot: bool,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct IntentOutputList(Vec<IntentOutput>);

impl IntentOutputList {
    fn into_vec(self) -> Vec<IntentOutput> {
        self.0
    }
}

impl<'de> Deserialize<'de> for IntentOutputList {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OutputVisitor;

        impl<'de> Visitor<'de> for OutputVisitor {
            type Value = IntentOutputList;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a structured output or list of structured outputs")
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(IntentOutputList::default())
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(IntentOutputList::default())
            }

            fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let outputs: Vec<IntentOutput> =
                    Deserialize::deserialize(SeqAccessDeserializer::new(seq))?;
                Ok(IntentOutputList(outputs))
            }

            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let output: IntentOutput =
                    Deserialize::deserialize(MapAccessDeserializer::new(map))?;
                Ok(IntentOutputList(vec![output]))
            }
        }

        deserializer.deserialize_any(OutputVisitor)
    }
}

#[derive(Debug, Clone)]
pub struct IntentConfig {
    intents: Vec<IntentDefinition>,
}

impl IntentConfig {
    fn from_path(path: Option<&Path>) -> Option<Self> {
        let path = path?;
        let raw = fs::read_to_string(path).ok()?;
        Self::from_yaml(&raw)
    }

    fn from_yaml(raw: &str) -> Option<Self> {
        let intents = load_intent_definitions(raw)?;
        Some(Self { intents })
    }

    pub fn detect(&self, prompt: &str) -> Option<AgentIntentMetadata> {
        self.intents
            .iter()
            .find(|intent| intent.matches(prompt))
            .map(|intent| intent.to_metadata(prompt))
    }
}

impl Default for IntentConfig {
    fn default() -> Self {
        let definition = IntentDefinition {
            id: "search_market_info".to_string(),
            triggers: vec![
                "搜行情".to_string(),
                "行情".to_string(),
                "stock quote".to_string(),
                "market index".to_string(),
                "A股".to_string(),
            ],
            primary_goal: Some("Collect the latest market index snapshot".to_string()),
            target_sites: vec![
                "https://www.google.com".to_string(),
                "https://www.baidu.com".to_string(),
            ],
            required_outputs: vec![IntentOutput {
                schema: "market_info_v1.json".to_string(),
                include_screenshot: true,
                description: Some("List indices with latest value and change".to_string()),
            }],
            preferred_language: Some("zh-CN".to_string()),
            blockers: HashMap::from([
                (
                    "consent_gate".to_string(),
                    "accept_google_consent".to_string(),
                ),
                ("unusual_traffic".to_string(), "switch_to_baidu".to_string()),
                ("captcha".to_string(), "require_manual_captcha".to_string()),
                (
                    "permission_request".to_string(),
                    "ack_permission_prompt".to_string(),
                ),
                (
                    "download_prompt".to_string(),
                    "wait_download_complete".to_string(),
                ),
                ("blank_page".to_string(), "auto_retry".to_string()),
            ]),
        };
        Self {
            intents: vec![definition],
        }
    }
}

fn contains_cjk(input: &str) -> bool {
    input
        .chars()
        .any(|ch| matches!(ch as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x20000..=0x2EBE0))
}

fn load_intent_definitions(raw: &str) -> Option<Vec<IntentDefinition>> {
    let yaml: YamlValue = serde_yaml::from_str(raw).ok()?;
    let intents_value = match yaml {
        YamlValue::Mapping(map) => map
            .get(&YamlValue::String("intents".to_string()))
            .cloned()
            .unwrap_or(YamlValue::Sequence(Vec::new())),
        _ => YamlValue::Sequence(Vec::new()),
    };
    Some(parse_intent_entries(intents_value))
}

fn parse_intent_entries(value: YamlValue) -> Vec<IntentDefinition> {
    match value {
        YamlValue::Sequence(entries) => entries
            .into_iter()
            .filter_map(|entry| parse_intent_spec(entry, None))
            .collect(),
        YamlValue::Mapping(map) => map
            .into_iter()
            .filter_map(|(key, value)| {
                if let Some(key) = key.as_str() {
                    parse_intent_spec(value, Some(key))
                } else {
                    warn!("intent key must be a string; skipping entry");
                    None
                }
            })
            .collect(),
        other => {
            warn!("intents entry must be a list or mapping, got {:?}", other);
            Vec::new()
        }
    }
}

fn parse_intent_spec(value: YamlValue, fallback_id: Option<&str>) -> Option<IntentDefinition> {
    match serde_yaml::from_value::<IntentDefinitionSpec>(value) {
        Ok(spec) => spec.into_definition(fallback_id).or_else(|| {
            warn!("intent entry missing id; skipping");
            None
        }),
        Err(err) => {
            warn!("failed to parse intent entry: {}", err);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soulbrowser_core_types::TaskId;

    #[test]
    fn detects_default_search_intent() {
        let config = IntentConfig::default();
        let intent = config
            .detect("帮我搜行情，看看今天A股变化")
            .expect("intent detected");
        assert_eq!(intent.intent_id.as_deref(), Some("search_market_info"));
        assert!(!intent.target_sites.is_empty());
        assert_eq!(intent.preferred_language.as_deref(), Some("zh-CN"));
        assert_eq!(intent.required_outputs[0].schema, "market_info_v1.json");
    }

    #[test]
    fn cjk_detection_handles_ascii() {
        assert!(contains_cjk("行情"));
        assert!(!contains_cjk("market"));
    }

    #[test]
    fn parses_map_based_intent_config() {
        let yaml = r#"
intents:
  summarize_news:
    triggers: ["news summary"]
    primary_goal: "Summarize tech headlines"
    primary_sites:
      - https://news.google.com
    output:
      schema: news_brief_v1.json
      include_screenshot: false
      description: Latest headlines
    preferred_language: zh-CN
    blockers:
      consent_gate: accept_google_consent
"#;
        let config = IntentConfig::from_yaml(yaml).expect("parse config");
        let intent = config
            .detect("帮我做一个 news summary")
            .expect("intent detected");
        assert_eq!(intent.intent_id.as_deref(), Some("summarize_news"));
        assert_eq!(intent.target_sites[0], "https://news.google.com");
        assert_eq!(intent.required_outputs[0].schema, "news_brief_v1.json");
    }

    #[test]
    fn todo_snapshot_lists_goal_sites_outputs() {
        let mut request = AgentRequest::new(TaskId::new(), "搜行情");
        request.intent.primary_goal = Some("查看今天A股行情".to_string());
        request.intent.target_sites = vec![
            "https://www.google.com".to_string(),
            "https://www.baidu.com".to_string(),
        ];
        let mut output = RequestedOutput::new("market_info_v1.json");
        output.include_screenshot = true;
        output.description = Some("最新指数".to_string());
        request.intent.required_outputs = vec![output];
        request.intent.preferred_language = Some("zh-CN".to_string());
        request.constraints = vec!["优先使用中文数据源".to_string()];

        let snapshot = build_todo_snapshot(&request).expect("todo snapshot");
        assert!(snapshot.contains("达成目标"));
        assert!(snapshot.contains("第1优先站点"));
        assert!(snapshot.contains("market_info_v1.json"));
        assert!(snapshot.contains("需要截图"));
        assert!(snapshot.contains("遵循限制"));
        assert!(snapshot.contains("回复语言: zh-CN"));
    }
}
