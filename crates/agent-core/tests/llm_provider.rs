use agent_core::{AgentRequest, LlmProvider, MockLlmProvider};
use futures::executor::block_on;
use soulbrowser_core_types::TaskId;

fn build_request(goal: &str) -> AgentRequest {
    let mut request = AgentRequest::new(TaskId::new(), goal.to_string());
    request.push_tagged_user_turn(goal);
    request
}

#[test]
fn mock_provider_emits_deterministic_plan() {
    let provider = MockLlmProvider::default();
    let request = build_request("Open the landing page");

    let outcome = block_on(provider.plan(&request)).expect("plan");
    assert_eq!(outcome.plan.steps.len(), 1);
    assert!(outcome
        .explanations
        .first()
        .unwrap()
        .contains("Mock initial plan"));
}

#[test]
fn replan_includes_failure_reason() {
    let provider = MockLlmProvider::default();
    let request = build_request("Fill out the signup form");

    let initial = block_on(provider.plan(&request)).expect("plan");
    let failure_reason = "Timeout waiting for submit";
    let replanned =
        block_on(provider.replan(&request, &initial.plan, failure_reason)).expect("replan");

    assert_eq!(replanned.plan.steps.len(), 1);
    assert!(replanned
        .explanations
        .iter()
        .any(|item| item.contains(failure_reason)));
}
