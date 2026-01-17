use serde_json::{json, Value};
use soulbrowser_kernel_types::execution::StepExecutionReport;

#[derive(Clone, Debug, Default)]
pub struct AgentTimelineEntry {
    pub thinking: Option<String>,
    pub evaluation: Option<String>,
    pub memory: Option<String>,
    pub next_goal: Option<String>,
}

impl AgentTimelineEntry {
    pub fn from_agent_state(state: &Value) -> Self {
        let map = state.as_object();
        Self {
            thinking: map.and_then(|m| m.get("thinking")).and_then(Value::as_str).map(|s| s.to_string()),
            evaluation: map.and_then(|m| m.get("evaluation")).and_then(Value::as_str).map(|s| s.to_string()),
            memory: map.and_then(|m| m.get("memory")).and_then(Value::as_str).map(|s| s.to_string()),
            next_goal: map.and_then(|m| m.get("next_goal")).and_then(Value::as_str).map(|s| s.to_string()),
        }
    }
}

pub fn timeline_entry_for_step(report: &StepExecutionReport) -> Option<AgentTimelineEntry> {
    report
        .agent_state
        .as_ref()
        .map(|value| AgentTimelineEntry::from_agent_state(value))
        .filter(|entry| {
            entry.thinking.is_some()
                || entry.evaluation.is_some()
                || entry.memory.is_some()
                || entry.next_goal.is_some()
        })
}

pub fn overlay_for_entry(entry: &AgentTimelineEntry) -> Value {
    json!({
        "thinking": entry.thinking,
        "evaluation": entry.evaluation,
        "memory": entry.memory,
        "next_goal": entry.next_goal,
    })
}
