use agent_core::{AgentRequest, ConversationRole, ConversationTurn};
use serde_json::Value;

use crate::agent::{FlowExecutionReport, StepExecutionStatus};
use crate::intent::update_todo_snapshot;

#[cfg(test)]
use crate::agent::executor::{DispatchRecord, StepExecutionReport};
#[cfg(test)]
use crate::agent::ChatRunner;

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
    let message = format!(
        "{} Please generate an alternative plan that avoids this failure while still achieving the goal.",
        failure_summary
    );

    next_request.push_turn(ConversationTurn::new(ConversationRole::System, message));
    if let Some(summary) = observation_summary {
        let note = format!("Latest observation summary: {summary}");
        next_request.push_turn(ConversationTurn::new(ConversationRole::System, note));
        failure_summary.push_str(&format!(" Latest observation summary: {summary}."));
    }
    if let Some(kind) = blocker_kind {
        apply_blocker_guidance(kind, &mut next_request);
        next_request.metadata.insert(
            "registry_blocker_kind".to_string(),
            Value::String(kind.to_string()),
        );
    }
    if blocker_kind.is_none() {
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
    }
    next_request.push_tagged_user_turn("Please suggest a revised plan that can succeed.");
    next_request.sync_intent_metadata();
    update_todo_snapshot(&mut next_request);

    Some((next_request, failure_summary))
}

fn apply_blocker_guidance(kind: &str, request: &mut AgentRequest) {
    let Some(strategy) = request.intent.blocker_remediations.get(kind) else {
        return;
    };
    match strategy.as_str() {
        "switch_to_baidu" => {
            prioritize_site(request, "baidu");
            let hint = "Detected Google blocker; prefer Baidu for the next attempt.";
            request.push_turn(ConversationTurn::new(
                ConversationRole::System,
                hint.to_string(),
            ));
        }
        "require_manual_captcha" => {
            let hint =
                "Page requires CAPTCHA solving. Pause automation and ask for manual assistance.";
            request.push_turn(ConversationTurn::new(
                ConversationRole::System,
                hint.to_string(),
            ));
        }
        strategy_id => {
            let note = format!(
                "Blocker '{kind}' triggered strategy '{strategy_id}'. Adjust plan accordingly."
            );
            request.push_turn(ConversationTurn::new(ConversationRole::System, note));
        }
    }
}

fn prioritize_site(request: &mut AgentRequest, keyword: &str) {
    if request.intent.target_sites.len() <= 1 {
        return;
    }
    let sites = request.intent.target_sites.clone();
    let mut prioritized: Vec<String> = sites
        .iter()
        .filter(|site| site.to_ascii_lowercase().contains(keyword))
        .cloned()
        .collect();
    let mut others: Vec<String> = sites
        .into_iter()
        .filter(|site| !site.to_ascii_lowercase().contains(keyword))
        .collect();
    if prioritized.is_empty() {
        return;
    }
    prioritized.append(&mut others);
    request.intent.target_sites = prioritized;
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{
        ConversationRole as TestRole, MockLlmProvider, PlanToFlowOptions, PlannerConfig,
    };
    use soulbrowser_core_types::TaskId;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn failure_report() -> FlowExecutionReport {
        FlowExecutionReport {
            success: false,
            steps: vec![StepExecutionReport {
                step_id: "step-1".into(),
                title: "Do something".into(),
                status: StepExecutionStatus::Failed,
                attempts: 2,
                error: Some("Network timeout".into()),
                dispatches: Vec::<DispatchRecord>::new(),
            }],
        }
    }

    #[test]
    fn augment_request_adds_failure_context() {
        let mut request = AgentRequest::new(TaskId::new(), "test");
        request.push_tagged_user_turn("test");

        let report = failure_report();
        let (updated_request, summary) =
            augment_request_for_replan(&request, &report, 0, None, None, None).expect("augment");

        assert!(summary.contains("Network timeout"));
        assert!(updated_request.conversation.len() >= request.conversation.len() + 2);
        let last_turn = updated_request.conversation.last().unwrap();
        assert_eq!(last_turn.role, TestRole::User);
    }

    #[test]
    fn augment_request_handles_missing_failure_step() {
        let report = FlowExecutionReport {
            success: false,
            steps: vec![],
        };
        let mut request = AgentRequest::new(TaskId::new(), "test");
        request.push_tagged_user_turn("test");

        let (updated, summary) =
            augment_request_for_replan(&request, &report, 1, None, None, None).expect("augment");
        assert!(summary.contains("attempt 2"));
        assert!(updated.conversation.len() >= request.conversation.len() + 2);
    }

    #[tokio::test]
    async fn mock_llm_replan_loop_recovers_after_failure() {
        let provider = Arc::new(MockLlmProvider::default());
        let runner =
            ChatRunner::with_config(PlannerConfig::default(), PlanToFlowOptions::default())
                .with_llm_provider(provider);

        let agent_request = runner.request_from_prompt(
            "Collect latest release notes".to_string(),
            None,
            vec!["prefer official docs".to_string()],
        );
        let mut session = runner.plan(agent_request.clone()).await.expect("plan");
        let mut exec_request = agent_request.clone();
        let mut attempt = 0u32;
        let executor = MockFlowExecutor::new();

        loop {
            let report = executor.run(attempt);
            if report.success {
                assert_eq!(attempt, 1, "should succeed after one replan");
                break;
            }

            let (updated_request, summary) =
                augment_request_for_replan(&exec_request, &report, attempt, None, None, None)
                    .expect("replan context");
            exec_request = updated_request;

            let replanned = runner
                .replan(exec_request.clone(), &session.plan, &summary)
                .await
                .expect("replan succeed");

            assert!(replanned
                .plan
                .steps
                .iter()
                .any(|step| step.metadata.contains_key("replan_reason")));

            session = replanned;
            attempt += 1;
            assert!(attempt < 3, "replan loop should finish quickly");
        }
    }

    struct MockFlowExecutor;

    impl MockFlowExecutor {
        fn new() -> Self {
            Self
        }

        fn run(&self, attempt: u32) -> FlowExecutionReport {
            if attempt == 0 {
                FlowExecutionReport {
                    success: false,
                    steps: vec![StepExecutionReport {
                        step_id: "mock-step".into(),
                        title: "Mock action".into(),
                        status: StepExecutionStatus::Failed,
                        attempts: 1,
                        error: Some("synthetic failure".into()),
                        dispatches: Vec::<DispatchRecord>::new(),
                    }],
                }
            } else {
                FlowExecutionReport {
                    success: true,
                    steps: vec![StepExecutionReport {
                        step_id: "mock-step".into(),
                        title: "Mock action".into(),
                        status: StepExecutionStatus::Success,
                        attempts: 1,
                        error: None,
                        dispatches: Vec::<DispatchRecord>::new(),
                    }],
                }
            }
        }
    }

    #[test]
    fn blocker_guidance_prioritizes_sites() {
        let mut request = AgentRequest::new(TaskId::new(), "search info");
        request.intent.target_sites = vec![
            "https://www.google.com".to_string(),
            "https://www.baidu.com".to_string(),
        ];
        request.intent.blocker_remediations =
            HashMap::from([("unusual_traffic".to_string(), "switch_to_baidu".to_string())]);
        let report = failure_report();
        let (updated, _) =
            augment_request_for_replan(&request, &report, 0, None, Some("unusual_traffic"), None)
                .expect("augment");
        assert_eq!(
            updated.intent.target_sites.first().unwrap(),
            "https://www.baidu.com"
        );
    }
}
