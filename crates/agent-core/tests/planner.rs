use agent_core::{
    plan_to_flow, AgentLocator, AgentPlanner, AgentRequest, AgentScrollTarget, AgentToolKind,
    ConversationRole, ConversationTurn, PlanToFlowOptions, PlannerConfig, RuleBasedPlanner,
};
use soulbrowser_core_types::TaskId;

fn build_request(goal: &str) -> AgentRequest {
    let mut request = AgentRequest::new(TaskId::new(), goal.to_string());
    request.push_turn(ConversationTurn::new(ConversationRole::User, goal));
    request
}

#[test]
fn rule_based_planner_emits_navigation_and_click() {
    let planner = RuleBasedPlanner::new(PlannerConfig::default());
    let request = build_request("Go to https://example.com then click \"Sign in\"");

    let outcome = planner.draft_plan(&request).expect("plan");
    assert_eq!(outcome.plan.steps.len(), 2);

    assert!(matches!(
        outcome.plan.steps[0].tool.kind,
        AgentToolKind::Navigate { ref url } if url == "https://example.com"
    ));
    assert!(matches!(
        outcome.plan.steps[1].tool.kind,
        AgentToolKind::Click { .. }
    ));

    let flow_result = plan_to_flow(&outcome.plan, PlanToFlowOptions::default()).expect("flow");
    assert_eq!(flow_result.step_count, 2);
}

#[test]
fn planner_falls_back_to_note_when_no_actions() {
    let planner = RuleBasedPlanner::new(PlannerConfig::default());
    let request = build_request("Just think about automation safety");

    let outcome = planner.draft_plan(&request).expect("plan");
    assert_eq!(outcome.plan.steps.len(), 1);
    assert!(matches!(
        outcome.plan.steps[0].tool.kind,
        AgentToolKind::Custom { .. }
    ));
}

#[test]
fn planner_recognizes_scroll_instructions() {
    let planner = RuleBasedPlanner::new(PlannerConfig::default());

    let request_top = build_request("Scroll to the top of the page");
    let plan_top = planner.draft_plan(&request_top).expect("plan");
    assert!(matches!(
        plan_top.plan.steps[0].tool.kind,
        AgentToolKind::Scroll {
            target: AgentScrollTarget::Top
        }
    ));

    let request_pixels = build_request("Scroll down by 400 pixels then click 'Next'");
    let plan_pixels = planner.draft_plan(&request_pixels).expect("plan");
    assert_eq!(plan_pixels.plan.steps.len(), 2);
    match &plan_pixels.plan.steps[0].tool.kind {
        AgentToolKind::Scroll {
            target: AgentScrollTarget::Pixels(delta),
        } => assert_eq!(*delta, 400),
        other => panic!("expected pixel scroll, got {other:?}"),
    }
    match &plan_pixels.plan.steps[1].tool.kind {
        AgentToolKind::Click { locator } => match locator {
            AgentLocator::Text { content, .. } => assert_eq!(content, "Next"),
            _ => panic!("unexpected locator: {locator:?}"),
        },
        other => panic!("expected click, got {other:?}"),
    }
}
