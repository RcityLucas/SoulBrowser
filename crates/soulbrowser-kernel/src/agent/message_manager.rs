use chrono::{DateTime, Utc};
use serde::Serialize;

/// Describes an entry tracked inside the agent history timeline.
#[derive(Debug, Clone)]
pub struct HistoryItem {
    tag: String,
    label: Option<String>,
    content: String,
    recorded_at: DateTime<Utc>,
}

/// Maximum characters per history item content to avoid token explosion
const MAX_ITEM_CONTENT_CHARS: usize = 1500;

impl HistoryItem {
    pub fn new(tag: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            label: None,
            content: truncate_content(content.into(), MAX_ITEM_CONTENT_CHARS),
            recorded_at: Utc::now(),
        }
    }

    pub fn with_label(
        tag: impl Into<String>,
        label: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            tag: tag.into(),
            label: Some(label.into()),
            content: truncate_content(content.into(), MAX_ITEM_CONTENT_CHARS),
            recorded_at: Utc::now(),
        }
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn recorded_at(&self) -> DateTime<Utc> {
        self.recorded_at
    }

    pub fn render(&self) -> String {
        let body = self.content.trim();
        if let Some(label) = &self.label {
            format!(
                "<{tag} label=\"{label}\">\n{body}\n</{tag}>",
                tag = self.tag
            )
        } else {
            format!("<{tag}>\n{body}\n</{tag}>", tag = self.tag)
        }
    }

    pub fn snapshot(&self) -> HistoryItemSnapshot {
        HistoryItemSnapshot {
            tag: self.tag.clone(),
            label: self.label.clone(),
            content: self.content.clone(),
            recorded_at: self.recorded_at.to_rfc3339(),
        }
    }
}

/// Maintains BrowserUse-style history strings for planner prompts.
#[derive(Debug, Clone)]
pub struct MessageManager {
    initial_request: String,
    follow_ups: Vec<String>,
    history_items: Vec<HistoryItem>,
    read_state_index: usize,
    max_history_items: Option<usize>,
}

impl MessageManager {
    pub fn new(initial_task: impl Into<String>) -> Self {
        Self {
            initial_request: initial_task.into(),
            follow_ups: Vec::new(),
            history_items: Vec::new(),
            read_state_index: 0,
            max_history_items: None,
        }
    }

    pub fn with_max_history(mut self, limit: Option<usize>) -> Self {
        self.max_history_items = limit;
        self
    }

    pub fn push_follow_up(&mut self, text: impl Into<String>) {
        let trimmed = text.into();
        if trimmed.trim().is_empty() {
            return;
        }
        self.follow_ups.push(trimmed);
    }

    pub fn push_item(&mut self, item: HistoryItem) {
        self.history_items.push(item);
    }

    pub fn push_read_state(&mut self, content: impl Into<String>) {
        let index = self.read_state_index;
        self.read_state_index += 1;
        let tag = format!("read_state_{index}");
        self.history_items
            .push(HistoryItem::new(tag, content.into()));
    }

    pub fn is_empty(&self) -> bool {
        self.history_items.is_empty()
    }

    pub fn task_prompt(&self) -> String {
        let mut sections = Vec::new();
        sections.push(wrap_block("initial_user_request", &self.initial_request));
        for follow_up in &self.follow_ups {
            sections.push(wrap_block("follow_up_user_request", follow_up));
        }
        sections.join("\n")
    }

    pub fn agent_history_prompt(&self) -> Option<String> {
        if self.history_items.is_empty() {
            return None;
        }
        let items = self.truncated_history();
        Some(items.join("\n"))
    }

    fn truncated_history(&self) -> Vec<String> {
        match self.max_history_items {
            None => self.history_items.iter().map(HistoryItem::render).collect(),
            Some(limit) if limit == 0 => Vec::new(),
            Some(limit) => {
                if self.history_items.len() <= limit {
                    return self.history_items.iter().map(HistoryItem::render).collect();
                }
                let mut rendered = Vec::new();
                if let Some(first) = self.history_items.first() {
                    rendered.push(first.render());
                }
                let omitted = self.history_items.len().saturating_sub(limit);
                rendered.push(format!(
                    "<sys>[..., {omitted} previous steps omitted...]</sys>"
                ));
                let tail_len = limit.saturating_sub(1);
                let start = self.history_items.len().saturating_sub(tail_len);
                for item in self.history_items[start..].iter() {
                    rendered.push(item.render());
                }
                rendered
            }
        }
    }

    pub fn snapshot(&self) -> MessageManagerSnapshot {
        MessageManagerSnapshot {
            task_prompt: self.task_prompt(),
            agent_history_prompt: self.agent_history_prompt(),
            history_items: self
                .history_items
                .iter()
                .map(HistoryItem::snapshot)
                .collect(),
        }
    }
}

fn wrap_block(tag: &str, content: &str) -> String {
    format!("<{tag}>\n{}\n</{tag}>", content.trim())
}

fn truncate_content(text: String, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text;
    }
    let mut truncated = String::with_capacity(max_chars + 20);
    truncated.push_str(&text[..max_chars]);
    truncated.push_str("... [truncated]");
    truncated
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageManagerSnapshot {
    pub task_prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_history_prompt: Option<String>,
    pub history_items: Vec<HistoryItemSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryItemSnapshot {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub content: String,
    pub recorded_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_prompt_wraps_initial_and_followups() {
        let mut manager = MessageManager::new("Find silver price");
        manager.push_follow_up("Check alternative sources");
        let prompt = manager.task_prompt();
        assert!(prompt.contains("<initial_user_request>"));
        assert!(prompt.contains("</initial_user_request>"));
        assert!(prompt.contains("<follow_up_user_request>"));
        assert!(prompt.contains("Check alternative sources"));
    }

    #[test]
    fn history_prompt_respects_limit() {
        let mut manager = MessageManager::new("Task").with_max_history(Some(3));
        manager.push_item(HistoryItem::new("sys", "init"));
        manager.push_item(HistoryItem::new("action", "click quote"));
        manager.push_item(HistoryItem::new("evaluation", "Page still 404"));
        manager.push_item(HistoryItem::new("action", "search fallback"));
        let prompt = manager.agent_history_prompt().unwrap();
        assert!(prompt.contains("previous steps omitted"));
        assert!(prompt.matches("<action>").count() >= 1);
    }

    #[test]
    fn read_state_items_get_unique_tags() {
        let mut manager = MessageManager::new("Task");
        manager.push_read_state("First DOM");
        manager.push_read_state("Second DOM");
        let prompt = manager.agent_history_prompt();
        assert!(prompt.unwrap().contains("<read_state_0>"));
    }
}
