use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub id: String,
    pub label: String,
    pub description: String,
    pub usage: String,
    pub required_fields: Vec<String>,
    pub optional_fields: Vec<String>,
    pub output: Option<String>,
    pub example: Option<Value>,
    pub priority: u8,
}

impl ToolDescriptor {
    pub fn new(
        id: &str,
        label: &str,
        description: &str,
        usage: &str,
        required_fields: &[&str],
        optional_fields: &[&str],
        output: Option<&str>,
        example: Option<Value>,
        priority: u8,
    ) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            description: description.to_string(),
            usage: usage.to_string(),
            required_fields: required_fields.iter().map(|s| s.to_string()).collect(),
            optional_fields: optional_fields.iter().map(|s| s.to_string()).collect(),
            output: output.map(|s| s.to_string()),
            example,
            priority,
        }
    }

    fn prompt_block(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "• {} ({}): {}",
            self.id, self.label, self.description
        ));
        lines.push(format!("  When to use: {}", self.usage));
        if !self.required_fields.is_empty() {
            lines.push(format!(
                "  Required fields: {}",
                self.required_fields.join(", ")
            ));
        }
        if !self.optional_fields.is_empty() {
            lines.push(format!(
                "  Optional fields: {}",
                self.optional_fields.join(", ")
            ));
        }
        if let Some(output) = &self.output {
            lines.push(format!("  Output: {}", output));
        }
        if let Some(example) = &self.example {
            lines.push(format!("  Example: {}", example));
        }
        lines.join("\n")
    }
}

#[derive(Debug, Error)]
pub enum ToolRegistryError {
    #[error("IO error while loading tool definitions: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse tool definitions: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Tool definition must be a JSON object or array of objects")]
    InvalidFormat,
}

#[derive(Debug)]
pub struct ToolRegistry {
    entries: RwLock<Vec<ToolDescriptor>>,
}

impl ToolRegistry {
    pub fn with_builtin_tools() -> Self {
        let registry = Self {
            entries: RwLock::new(Vec::new()),
        };
        for descriptor in builtin_descriptors() {
            registry.register(descriptor);
        }
        registry
    }

    pub fn register(&self, descriptor: ToolDescriptor) {
        let mut guard = self.entries.write();
        if let Some(existing) = guard.iter_mut().find(|entry| entry.id == descriptor.id) {
            *existing = descriptor;
        } else {
            guard.push(descriptor);
        }
        guard.sort_by(|a, b| match a.priority.cmp(&b.priority) {
            Ordering::Equal => a.id.cmp(&b.id),
            other => other,
        });
    }

    pub fn register_many(&self, descriptors: &[ToolDescriptor]) -> usize {
        for descriptor in descriptors {
            self.register(descriptor.clone());
        }
        descriptors.len()
    }

    pub fn remove(&self, id: &str) -> bool {
        let mut guard = self.entries.write();
        let before = guard.len();
        guard.retain(|entry| entry.id != id);
        before != guard.len()
    }

    pub fn list(&self) -> Vec<ToolDescriptor> {
        self.entries.read().clone()
    }

    pub fn prompt_for_llm(&self, max_entries: usize) -> Option<String> {
        if max_entries == 0 {
            return None;
        }
        let guard = self.entries.read();
        if guard.is_empty() {
            return None;
        }
        let mut blocks = Vec::new();
        for descriptor in guard.iter().take(max_entries) {
            blocks.push(descriptor.prompt_block());
        }
        Some(blocks.join("\n"))
    }

    pub fn load_from_path<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> Result<Vec<ToolDescriptor>, ToolRegistryError> {
        let raw = fs::read_to_string(path)?;
        let descriptors = parse_tool_definitions(&raw)?;
        self.register_many(&descriptors);
        Ok(descriptors)
    }

    pub fn load_from_dir<P: AsRef<Path>>(&self, dir: P) -> Result<usize, ToolRegistryError> {
        let dir = dir.as_ref();
        if !dir.exists() {
            return Ok(0);
        }
        let mut loaded = 0;
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() {
                continue;
            }
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("json"))
                .unwrap_or(false)
            {
                let count = self.load_from_path(&path)?.len();
                loaded += count;
            }
        }
        Ok(loaded)
    }
}

fn builtin_descriptors() -> Vec<ToolDescriptor> {
    use serde_json::json;
    vec![
        ToolDescriptor::new(
            "navigate-to-url",
            "Navigate",
            "Open a URL in the active tab with built-in wait tiers.",
            "Use to load a new page before interacting with it.",
            &["url"],
            &["wait_tier"],
            Some("Action waits for DOM ready or idle state."),
            Some(json!({ "url": "https://example.com", "wait_tier": "dom_ready" })),
            0,
        ),
        ToolDescriptor::new(
            "click",
            "Click",
            "Click an element located via CSS, text, or ARIA selectors.",
            "Use after the target element becomes visible.",
            &["anchor"],
            &["wait_tier"],
            Some("Returns whether the click succeeded."),
            Some(json!({ "anchor": { "strategy": "css", "selector": "button.submit" } })),
            1,
        ),
        ToolDescriptor::new(
            "type-text",
            "Type Text",
            "Focus an input and type characters, optionally submitting the form.",
            "Use for search fields or forms that need textual input.",
            &["anchor", "text"],
            &["submit", "wait_tier"],
            Some("Reports that the keystrokes were dispatched."),
            Some(json!({
                "anchor": { "strategy": "css", "selector": "input[name=q]" },
                "text": "白银 走势",
                "submit": true
            })),
            2,
        ),
        ToolDescriptor::new(
            "select-option",
            "Select",
            "Choose a value from a dropdown or list widget.",
            "Use when a select control must change value before the next step.",
            &["anchor", "value"],
            &["method", "wait_tier"],
            Some("Indicates which option was selected."),
            Some(json!({
                "anchor": { "strategy": "css", "selector": "select#currency" },
                "value": "XAG"
            })),
            3,
        ),
        ToolDescriptor::new(
            "scroll-page",
            "Scroll",
            "Scroll the viewport or a container to reveal new content.",
            "Use to load lazy content or reach sticky footers.",
            &["target"],
            &["behavior"],
            Some("Reports the scroll target that was applied."),
            Some(json!({ "target": { "kind": "pixels", "value": 600 } })),
            4,
        ),
        ToolDescriptor::new(
            "wait-for-element",
            "Wait For Element",
            "Wait for an element to appear or disappear before proceeding.",
            "Use after navigation or actions where DOM timing is uncertain.",
            &["target", "condition"],
            &[],
            Some("Returns when the condition is satisfied."),
            Some(json!({
                "target": { "strategy": "css", "selector": "div#content_left" },
                "condition": { "kind": "visible" }
            })),
            5,
        ),
        ToolDescriptor::new(
            "browser.search",
            "Web Search",
            "Open a Baidu/Bing/Google results page for a query (optional site filter).",
            "Use to discover new sources when fixed URLs fail or when guidance says 'use search'.",
            &["query"],
            &["engine", "site", "results_selector"],
            Some("Navigates to the results page and waits for the results container."),
            Some(json!({ "query": "东方财富 白银", "engine": "baidu" })),
            6,
        ),
        ToolDescriptor::new(
            "browser.search.click-result",
            "Search Result Click",
            "Auto-select SERP links that match guardrail domains and click them immediately.",
            "Injected after browser.search so AutoAct can enter authority sites without waiting for a new LLM step.",
            &[],
            &["engine", "selectors", "domains"],
            Some("Reports whether a guardrail domain was matched or if a fallback result was used."),
            Some(json!({ "engine": "baidu", "domains": ["chinacourt.gov.cn"] })),
            7,
        ),
        ToolDescriptor::new(
            "browser.close-modal",
            "Close Modal",
            "Dismiss popups by clicking close controls or sending ESC as a fallback.",
            "Use when modals block interaction with the main page.",
            &[],
            &["selector", "selectors", "focus_selector", "fallback_escape"],
            Some("Reports whether a close selector or Escape key was used."),
            Some(json!({ "selectors": ["button[aria-label=关闭]", ".modal-close"] })),
            8,
        ),
        ToolDescriptor::new(
            "browser.send-esc",
            "Send Escape",
            "Dispatch Escape key events to dismiss dialogs or exit inputs.",
            "Use to cancel lightboxes, dropdowns, or sticky overlays.",
            &[],
            &["count", "focus_selector"],
            Some("Indicates whether any target accepted the key event."),
            Some(json!({ "count": 2 })),
            9,
        ),
        ToolDescriptor::new(
            "weather.search",
            "Weather Search",
            "Macro that searches Baidu for weather widgets and follows trusted links.",
            "Use for weather intents before parsing the widget output.",
            &["query"],
            &["search_url", "preferred_link_substring"],
            Some("Returns the final destination URL and captured snapshot."),
            Some(json!({ "query": "北京 天气" })),
            10,
        ),
        ToolDescriptor::new(
            "data.extract-site",
            "Extract Site",
            "Capture the current DOM snapshot for downstream parsing.",
            "Use before any data.parse.* tool that needs HTML context.",
            &[],
            &[],
            Some("Outputs structured title/text samples for parsers."),
            None,
            11,
        ),
        ToolDescriptor::new(
            "market.quote.fetch",
            "Market Quote Fetch",
            "Query configured quote sources (DOM or API) for the requested metal.",
            "Use for market intents that expect structured price tables.",
            &[],
            &["source_url"],
            Some("Returns normalized quote rows for downstream parsing."),
            None,
            12,
        ),
        ToolDescriptor::new(
            "data.validate-target",
            "Validate Target Page",
            "Check the captured page snapshot for required keywords, allowed domains, and status codes.",
            "Use immediately after navigating/observing to ensure the page really matches the requested market source before parsing.",
            &["payload.keywords", "payload.allowed_domains"],
            &["payload.expected_status", "payload.source_step_id"],
            Some("Returns matched keywords, domain, and HTTP status when validation succeeds."),
            Some(json!({
                "payload": {
                    "keywords": ["白银", "行情"],
                    "allowed_domains": ["quote.eastmoney.com"],
                    "expected_status": 200,
                    "source_step_id": "observe-1"
                }
            })),
            13,
        ),
        ToolDescriptor::new(
            "data.parse.generic",
            "Parse Generic Observation",
            "Summarize arbitrary DOM observations into generic_observation_v1 schema.",
            "Use when no specialized parser fits the data type.",
            &["source_step_id"],
            &[],
            Some("Outputs generic_observation_v1 JSON."),
            None,
            14,
        ),
        ToolDescriptor::new(
            "data.deliver.structured",
            "Deliver Structured Artifact",
            "Persist the parsed payload as a user-facing artifact with schema metadata.",
            "Use after parsing to hand data back to the caller.",
            &["schema", "filename", "artifact_label", "source_step_id"],
            &[],
            Some("Writes a JSON artifact linked to the source parse step."),
            None,
            15,
        ),
    ]
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::with_builtin_tools()
    }
}

pub fn default_tool_registry() -> Arc<ToolRegistry> {
    Arc::new(ToolRegistry::with_builtin_tools())
}

fn parse_tool_definitions(raw: &str) -> Result<Vec<ToolDescriptor>, ToolRegistryError> {
    let value: Value = serde_json::from_str(raw)?;
    match value {
        Value::Array(items) => items
            .into_iter()
            .map(|item| parse_tool_value(item))
            .collect(),
        Value::Object(_) => Ok(vec![parse_tool_value(value)?]),
        _ => Err(ToolRegistryError::InvalidFormat),
    }
}

fn parse_tool_value(value: Value) -> Result<ToolDescriptor, ToolRegistryError> {
    let definition: ToolDefinition = serde_json::from_value(value)?;
    Ok(definition.into())
}

#[derive(Debug, Deserialize)]
struct ToolDefinition {
    pub id: String,
    pub label: String,
    pub description: String,
    pub usage: String,
    #[serde(default)]
    pub required_fields: Vec<String>,
    #[serde(default)]
    pub optional_fields: Vec<String>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub example: Option<Value>,
    #[serde(default)]
    pub priority: Option<u8>,
}

impl From<ToolDefinition> for ToolDescriptor {
    fn from(value: ToolDefinition) -> Self {
        ToolDescriptor {
            id: value.id,
            label: value.label,
            description: value.description,
            usage: value.usage,
            required_fields: value.required_fields,
            optional_fields: value.optional_fields,
            output: value.output,
            example: value.example,
            priority: value.priority.unwrap_or(200),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn prompt_includes_builtins() {
        let registry = ToolRegistry::with_builtin_tools();
        let prompt = registry.prompt_for_llm(10).expect("prompt");
        assert!(prompt.contains("navigate-to-url"));
        assert!(prompt.contains("browser.search"));
    }

    #[test]
    fn register_overwrites_existing() {
        let registry = ToolRegistry::with_builtin_tools();
        registry.register(ToolDescriptor::new(
            "browser.search",
            "Web Search (custom)",
            "Custom description",
            "Custom usage",
            &[],
            &[],
            None,
            None,
            0,
        ));
        let prompt = registry.prompt_for_llm(1).expect("prompt");
        assert!(prompt.contains("Custom description"));
    }

    #[test]
    fn parse_single_definition() {
        let json = r#"{"id":"custom.tool","label":"Test","description":"desc","usage":"use","required_fields":["url"],"optional_fields":[],"priority":10}"#;
        let defs = parse_tool_definitions(json).expect("parse");
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].id, "custom.tool");
        assert_eq!(defs[0].priority, 10);
    }

    #[test]
    fn load_from_dir_registers_entries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("tool.json");
        let mut file = File::create(&path).expect("file");
        writeln!(
            file,
            "[{{\"id\":\"custom.a\",\"label\":\"A\",\"description\":\"d\",\"usage\":\"when\"}}]"
        )
        .expect("write");
        let registry = ToolRegistry::with_builtin_tools();
        registry.load_from_dir(tmp.path()).expect("load from dir");
        assert!(registry.list().iter().any(|t| t.id == "custom.a"));
    }

    #[test]
    fn remove_tool_from_registry() {
        let registry = ToolRegistry::with_builtin_tools();
        registry.register(ToolDescriptor::new(
            "custom.remove",
            "Remove",
            "desc",
            "use",
            &[],
            &[],
            None,
            None,
            5,
        ));
        assert!(registry.remove("custom.remove"));
        assert!(!registry.list().iter().any(|t| t.id == "custom.remove"));
    }
}
