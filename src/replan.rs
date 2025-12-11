use agent_core::{AgentRequest, ConversationRole, ConversationTurn};
use serde_json::Value;

use crate::agent::{FlowExecutionReport, StepExecutionStatus};
use crate::intent::update_todo_snapshot;

/// Enrich the agent request with failure context so LLM planners can replan intelligently.
pub fn augment_request_for_replan(
    request: &AgentRequest,
    report: &FlowExecutionReport,
    attempt: u32,
    observation_summary: Option<&str>,
    blocker_kind: Option<&str>,
    agent_history_prompt: Option<&str>,
) -> Option<(AgentRequest, String)> {
    let failure_step = report
        .steps
        .iter()
        .rev()
        .find(|step| matches!(step.status, StepExecutionStatus::Failed));

    let mut next_request = request.clone();
    let mut failure_summary = if let Some(step) = failure_step {
        let error = step.error.as_deref().unwrap_or("unknown error");
        format!(
            "Execution attempt {} failed at step '{}' after {} attempt(s). Error: {}.",
            attempt + 1,
            step.title,
            step.attempts,
            error
        )
    } else {
        format!(
            "Execution attempt {} failed for unspecified reasons.",
            attempt + 1
        )
    };

    let prompt = format!(
        "{} Please generate an alternative plan that avoids this failure while still achieving the goal.",
        failure_summary
    );
    next_request.push_turn(ConversationTurn::new(ConversationRole::System, prompt));

    if let Some(summary) = observation_summary {
        let note = format!("Latest observation summary: {summary}");
        next_request.push_turn(ConversationTurn::new(
            ConversationRole::System,
            note.clone(),
        ));
        failure_summary.push_str(&format!(" Latest observation summary: {summary}."));
    }

    if let Some(kind) = blocker_kind {
        apply_blocker_guidance(kind, &mut next_request);
        next_request.metadata.insert(
            "registry_blocker_kind".to_string(),
            Value::String(kind.to_string()),
        );
    } else {
        next_request.metadata.remove("registry_blocker_kind");
    }

    if let Some(history) = agent_history_prompt {
        let history_block = history.to_string();
        next_request.push_turn(ConversationTurn::new(
            ConversationRole::System,
            history_block.clone(),
        ));
        next_request.metadata.insert(
            "agent_history_prompt".to_string(),
            Value::String(history_block),
        );
    } else {
        next_request.metadata.remove("agent_history_prompt");
    }

    next_request.push_turn(ConversationTurn::new(
        ConversationRole::User,
        "Please suggest a revised plan that can succeed.".to_string(),
    ));
    update_todo_snapshot(&mut next_request);

    Some((next_request, failure_summary))
}

fn apply_blocker_guidance(kind: &str, request: &mut AgentRequest) {
    let note = format!(
        "Blocker '{kind}' was observed during the last execution. Adjust the new plan accordingly."
    );
    request.push_turn(ConversationTurn::new(ConversationRole::System, note));
}
