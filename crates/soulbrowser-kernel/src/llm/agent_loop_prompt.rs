use agent_core::{
    format_system_prompt, format_user_message, AgentError, AgentHistoryEntry, AgentOutput,
    AgentRequest, BrowserStateSummary,
};

use crate::llm::utils::extract_json_object;

/// Build the system prompt for agent loop decisions.
pub fn build_system_prompt(state: &BrowserStateSummary) -> String {
    format_system_prompt(state.screenshot_base64.is_some())
}

/// Build the user message that includes the current browser state and task.
pub fn build_user_message(
    request: &AgentRequest,
    state: &BrowserStateSummary,
    history: &[AgentHistoryEntry],
) -> String {
    format_user_message(request, state, history)
}

/// Parse the raw LLM output into an [`AgentOutput`].
pub fn parse_agent_output(raw: &str) -> Result<AgentOutput, AgentError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AgentError::invalid_request(
            "LLM response missing agent loop payload",
        ));
    }

    let json_candidate = extract_json_object(trimmed).unwrap_or_else(|| trimmed.to_string());
    serde_json::from_str(&json_candidate).map_err(|err| {
        AgentError::invalid_request(format!("Failed to parse agent loop JSON response: {}", err))
    })
}
