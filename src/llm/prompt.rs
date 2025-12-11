use agent_core::{AgentPlan, AgentRequest};
use serde_json::Value;

const CAPABILITY_OVERVIEW: &str = "- Browser automation core: navigate, click, type_text (with submit), select, scroll, wait (visible/hidden/network idle).\n- Observation -> Parse -> Deliver pipeline is enforced automatically; structured outputs always flow through `data.extract-site` -> `data.parse.*` -> `data.deliver.structured`.\n- Deterministic parsers available: generic observation, market info, news brief, GitHub repositories, Twitter feed, Facebook feed, LinkedIn profile, Hacker News feed.\n- GitHub repo parsing can auto-fill the username based on recent navigation/current URL if planner forgets.\n- Planner may insert `agent.note` for inline reporting when needed.\n- The executor automatically normalizes tool aliases (e.g., browser.*) and enforces sensible waits/timeouts.\n";

pub struct PromptBuilder;

impl PromptBuilder {
    pub fn new() -> Self {
        Self
    }

    pub fn system_prompt(&self) -> &'static str {
        "You are SoulBrowser's planning strategist. Always read the structured channels (e.g., <initial_user_request>, <follow_up_user_request>, <agent_history>, <browser_state>, <browser_vision>, <read_state>, <file_system>, <todo_md>) before reasoning. Generate deterministic JSON plans for web automation that turn those requirements into concrete actions."
    }

    pub fn build_user_prompt(
        &self,
        request: &AgentRequest,
        previous_plan: Option<&AgentPlan>,
        failure_summary: Option<&str>,
    ) -> String {
        let mut sections = Vec::new();
        sections.push(format!("System capabilities:\n{}", CAPABILITY_OVERVIEW));
        sections.push(format!("Goal: {}", request.goal.trim()));
        if let Some(primary_goal) = request.intent.primary_goal.as_ref() {
            if primary_goal.trim() != request.goal.trim() {
                sections.push(format!("Primary intent goal: {}", primary_goal.trim()));
            }
        }
        if let Some(intent_id) = request.intent.intent_id.as_ref() {
            sections.push(format!("Intent id: {intent_id}"));
        }
        if !request.constraints.is_empty() {
            sections.push(format!("Constraints: {}", request.constraints.join(", ")));
        }
        if let Some(ctx) = request.context.as_ref() {
            if let Some(url) = ctx.current_url.as_ref() {
                sections.push(format!("Current URL: {url}"));
            }
        }
        if !request.intent.target_sites.is_empty() {
            sections.push(format!(
                "Target sites (ordered): {}",
                request.intent.target_sites.join(" -> ")
            ));
        }
        if !request.intent.required_outputs.is_empty() {
            let outputs = request
                .intent
                .required_outputs
                .iter()
                .map(|output| {
                    let mut parts = vec![format!("schema={}", output.schema)];
                    if let Some(desc) = output.description.as_ref() {
                        parts.push(desc.clone());
                    }
                    if output.include_screenshot {
                        parts.push("needs_screenshot".to_string());
                    }
                    parts.join(" | ")
                })
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("Required structured outputs:\n{}", outputs));
        }
        if let Some(language) = request.intent.preferred_language.as_ref() {
            sections.push(format!("Preferred language: {language}"));
        }
        if !request.intent.blocker_remediations.is_empty() {
            let blockers = request
                .intent
                .blocker_remediations
                .iter()
                .map(|(kind, remediation)| format!("- {kind}: {remediation}"))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("Blocker remediation map:\n{}", blockers));
        }
        if !request.conversation.is_empty() {
            let history = request
                .conversation
                .iter()
                .rev()
                .take(4)
                .rev()
                .map(|turn| format!("- {:?}: {}", turn.role, turn.message.trim()))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("Recent conversation:\n{}", history));
        }
        if let Some(history_block) = request
            .metadata
            .get("agent_history_prompt")
            .and_then(Value::as_str)
        {
            sections.push(history_block.to_string());
        }
        if let Some(helpers) = request
            .metadata
            .get("registry_helper_prompt")
            .and_then(Value::as_str)
        {
            sections.push(format!("Registry helper actions:\n{}", helpers));
        }

        if let Some(browser_state) = browser_state_section(request) {
            sections.push(browser_state);
        }

        if let Some(browser_vision) = browser_vision_section(request) {
            sections.push(browser_vision);
        }

        if let Some(read_state) = read_state_section(request) {
            sections.push(read_state);
        }

        if let Some(file_state) = file_system_section(request) {
            sections.push(file_state);
        }

        if let Some(todo_md) = todo_section(request) {
            sections.push(todo_md);
        }

        if let Some(plan) = previous_plan {
            sections.push(format!("Previous plan summary:\n{}", summarize_plan(plan)));
        }
        if let Some(summary) = failure_summary {
            sections.push(format!("Failure context: {}", summary));
        }

        format!(
            "{header}\n\nProduce JSON following this schema: {schema}\nRules:\n1. Always emit valid JSON without markdown.\n2. Use css selectors prefixed with 'css=' when possible.\n3. Supported actions: navigate, click, type_text, select, scroll, wait.\n4. Use wait='idle' for network idle waits.\n5. Provide rationale and risks arrays even if empty.\n6. When structured outputs are requested, include explicit navigate -> observe -> act -> parse -> deliver steps covering data acquisition, parsing, and persistence.\n7. Parsing steps must cite deterministic parsers and final steps must mention the structured schema/screenshot that will be produced.\n8. Custom tool allowlist (anything else will be rejected): data.extract-site (observation), data.parse.market_info, data.parse.news_brief, data.parse.twitter-feed, data.parse.facebook-feed, data.parse.linkedin-profile, data.parse.hackernews-feed, data.parse.github-repo (or github.extract-repo alias), data.deliver.structured, and agent.note.\n9. `data.parse.github-repo` steps must include `payload.username` (GitHub handle without the leading `@`) derived from the navigation target or user request.\n10. To observe a page, rely on the automatically inserted data.extract-site step; do not schedule extra ad-hoc \"observe\" actions.\n",
            header = sections.join("\n"),
            schema = JSON_SCHEMA
        )
    }
}

fn summarize_plan(plan: &AgentPlan) -> String {
    plan.steps
        .iter()
        .enumerate()
        .map(|(index, step)| format!("{}. {} - {}", index + 1, step.title, step.detail))
        .collect::<Vec<_>>()
        .join("\n")
}

fn browser_state_section(request: &AgentRequest) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(ctx) = request.context.as_ref() {
        if let Some(url) = ctx.current_url.as_deref() {
            lines.push(format!("Current URL: {url}"));
        }
    }

    if let Some(snapshot) = metadata_value(request, "browser_state_snapshot") {
        let mut structural = structural_lines(snapshot);
        lines.append(&mut structural);
    }

    wrap_block("browser_state", lines)
}

fn browser_vision_section(request: &AgentRequest) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(snapshot) = metadata_value(request, "browser_state_snapshot") {
        let mut vision = vision_lines(snapshot);
        lines.append(&mut vision);
    }
    wrap_block("browser_vision", lines)
}

fn structural_lines(snapshot: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(success) = snapshot.get("success").and_then(Value::as_bool) {
        lines.push(format!("Capture success: {}", success));
    }
    let root = snapshot_root(snapshot);

    if let Some(structural) = root.get("structural") {
        let mut parts = Vec::new();
        if let Some(nodes) = structural.get("dom_node_count").and_then(Value::as_u64) {
            parts.push(format!("nodes={}", nodes));
        }
        if let Some(interactive) = structural
            .get("interactive_element_count")
            .and_then(Value::as_u64)
        {
            parts.push(format!("interactive={}", interactive));
        }
        if let Some(forms) = structural.get("has_forms").and_then(Value::as_bool) {
            parts.push(format!("forms={}", forms));
        }
        if let Some(nav) = structural.get("has_navigation").and_then(Value::as_bool) {
            parts.push(format!("nav={}", nav));
        }
        if !parts.is_empty() {
            lines.push(format!("Structural: {}", parts.join(" ")));
        }
    }
    lines
}

fn vision_lines(snapshot: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if snapshot
        .get("screenshot_base64")
        .and_then(Value::as_str)
        .is_some()
    {
        lines.push("Screenshot captured: true".to_string());
    }
    let root = snapshot_root(snapshot);

    if let Some(semantic) = root.get("semantic") {
        let summary = semantic
            .get("summary")
            .and_then(Value::as_str)
            .map(|s| truncate_text(s.trim(), 160));
        let language = semantic.get("language").and_then(Value::as_str);
        if let Some(text) = summary {
            let mut line = format!("Semantic: {}", text);
            if let Some(lang) = language {
                line.push_str(&format!(" (lang={})", lang));
            }
            lines.push(line);
        }
    }

    if let Some(insights) = root.get("insights").and_then(Value::as_array) {
        for insight in insights.iter().take(2) {
            if let Some(desc) = insight.get("description").and_then(Value::as_str) {
                lines.push(format!("Insight: {}", truncate_text(desc.trim(), 120)));
            }
        }
    }

    lines
}

fn todo_section(request: &AgentRequest) -> Option<String> {
    let Some(value) = metadata_value(request, "todo_snapshot") else {
        return None;
    };

    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        return wrap_block("todo_md", vec![truncate_text(trimmed, 500)]);
    }

    if let Some(items) = value.as_array() {
        let mut bullets = Vec::new();
        for item in items.iter().take(20) {
            if let Some(text) = item.as_str() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    bullets.push(format!("- {}", truncate_text(trimmed, 160)));
                }
                continue;
            }
            if let Some(obj) = item.as_object() {
                let text = obj
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("todo item");
                let done = obj.get("done").and_then(Value::as_bool).unwrap_or(false);
                let prefix = if done { "[x]" } else { "[ ]" };
                bullets.push(format!("{} {}", prefix, truncate_text(text.trim(), 160)));
            }
        }
        if bullets.is_empty() {
            return None;
        }
        return wrap_block("todo_md", bullets);
    }

    None
}

fn read_state_section(request: &AgentRequest) -> Option<String> {
    let Some(value) = metadata_value(request, "read_state_snapshot") else {
        return None;
    };
    let lines = value_to_lines(value, 10, 160);
    wrap_block("read_state", lines)
}

fn file_system_section(request: &AgentRequest) -> Option<String> {
    let Some(value) = metadata_value(request, "file_system_snapshot") else {
        return None;
    };
    let lines = value_to_lines(value, 12, 160);
    wrap_block("file_system", lines)
}

fn metadata_value<'a>(request: &'a AgentRequest, key: &str) -> Option<&'a Value> {
    request
        .metadata
        .get(key)
        .or_else(|| request.context.as_ref()?.metadata.get(key))
}

fn wrap_block(tag: &str, lines: Vec<String>) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    let mut block = Vec::with_capacity(lines.len() + 2);
    block.push(format!("<{tag}>"));
    block.extend(lines);
    block.push(format!("</{tag}>"));
    Some(block.join("\n"))
}

fn snapshot_root<'a>(snapshot: &'a Value) -> &'a Value {
    snapshot.get("perception").unwrap_or(snapshot)
}

fn value_to_lines(value: &Value, max_items: usize, max_len: usize) -> Vec<String> {
    match value {
        Value::String(text) => vec![truncate_text(text.trim(), max_len)],
        Value::Array(items) => items
            .iter()
            .take(max_items)
            .filter_map(|item| match item {
                Value::String(text) => Some(format!("- {}", truncate_text(text.trim(), max_len))),
                Value::Object(map) => {
                    let summary = map
                        .get("text")
                        .and_then(Value::as_str)
                        .or_else(|| map.get("description").and_then(Value::as_str))
                        .unwrap_or("item");
                    Some(format!("- {}", truncate_text(summary.trim(), max_len)))
                }
                Value::Number(num) => Some(format!("- {}", num)),
                _ => {
                    let rendered = item.to_string();
                    Some(format!("- {}", truncate_text(&rendered, max_len)))
                }
            })
            .collect(),
        Value::Object(map) => map
            .iter()
            .take(max_items)
            .map(|(key, val)| {
                let text = if let Some(s) = val.as_str() {
                    truncate_text(s.trim(), max_len)
                } else {
                    let rendered = val.to_string();
                    truncate_text(&rendered, max_len)
                };
                format!("{key}: {text}")
            })
            .collect(),
        Value::Number(num) => vec![format!("{}", num)],
        Value::Bool(flag) => vec![format!("{}", flag)],
        Value::Null => Vec::new(),
    }
}

fn truncate_text(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        let mut shortened = text[..max_len].to_string();
        shortened.push('…');
        shortened
    }
}

const JSON_SCHEMA: &str = r#"{
  \"title\": \"High level plan title\",
  \"description\": \"Longer summary\",
  \"rationale\": [\"Reasoning bullet\"],
  \"risks\": [\"Potential risk\"],
  \"steps\": [
    {
      \"title\": \"Short name\",
      \"detail\": \"Specific instructions\",
      \"action\": \"navigate|click|type_text|select|scroll|wait\",
      \"url\": \"Required for navigate\",
      \"locator\": \"css=.selector or text=Submit\",
      \"text\": \"Input text when action=type_text\",
      \"value\": \"Option or wait duration\",
      \"target\": \"scroll target or wait hint\",
      \"wait\": \"dom_ready|idle|none\",
      \"timeout_ms\": 8000,
      \"validations\": [
        { \"description\": \"Ensure form submitted\", \"kind\": \"url_matches\", \"argument\": \"https://example.com/thanks\" }
      ]
    }
  ]
}"#;
