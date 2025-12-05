use agent_core::{
    AgentPlan, AgentPlanStep, AgentRequest, AgentTool, AgentToolKind, PlanValidator, WaitMode,
};
use serde_json::json;
use soulbrowser_core_types::TaskId;

fn navigation_step(url: &str) -> AgentPlanStep {
    AgentPlanStep::new(
        "step-1",
        "Navigate",
        AgentTool {
            kind: AgentToolKind::Navigate {
                url: url.to_string(),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    )
}

fn observe_step() -> AgentPlanStep {
    AgentPlanStep::new(
        "step-2",
        "Observe",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.extract-site".to_string(),
                payload: serde_json::json!({"navigate": false}),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    )
}

fn click_step() -> AgentPlanStep {
    AgentPlanStep::new(
        "step-3",
        "Click",
        AgentTool {
            kind: AgentToolKind::Click {
                locator: agent_core::AgentLocator::Css("button".to_string()),
            },
            wait: WaitMode::DomReady,
            timeout_ms: None,
        },
    )
}

fn parse_step() -> AgentPlanStep {
    AgentPlanStep::new(
        "step-4",
        "Parse",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.parse.market_info".to_string(),
                payload: json!({}),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    )
}

fn deliver_step() -> AgentPlanStep {
    AgentPlanStep::new(
        "step-5",
        "Deliver",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.deliver.structured".to_string(),
                payload: json!({ "schema": "market_info_v1" }),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    )
}

fn github_parse_step(payload: serde_json::Value) -> AgentPlanStep {
    AgentPlanStep::new(
        "step-github",
        "Parse GitHub repos",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.parse.github-repo".to_string(),
                payload,
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    )
}

#[test]
fn validator_rejects_wrong_target_site() {
    let mut request = AgentRequest::new(TaskId::new(), "Check quotes");
    request.intent.target_sites = vec!["baidu".to_string()];
    let mut plan = AgentPlan::new(request.task_id.clone(), "test");
    plan.push_step(navigation_step("https://www.google.com"));

    let validator = PlanValidator::default();
    assert!(validator.validate(&plan, &request).is_err());
}

#[test]
fn validator_accepts_structured_pipeline() {
    let mut request = AgentRequest::new(TaskId::new(), "Collect markets");
    request
        .intent
        .required_outputs
        .push(agent_core::RequestedOutput::new("market_info_v1.json"));

    let mut plan = AgentPlan::new(request.task_id.clone(), "pipeline");
    plan.push_step(navigation_step("https://www.baidu.com"));
    plan.push_step(observe_step());
    plan.push_step(click_step());
    plan.push_step(parse_step());
    plan.push_step(deliver_step());

    let validator = PlanValidator::default();
    assert!(validator.validate(&plan, &request).is_ok());
}

#[test]
fn validator_rejects_missing_deliver_step() {
    let mut request = AgentRequest::new(TaskId::new(), "Collect markets");
    request
        .intent
        .required_outputs
        .push(agent_core::RequestedOutput::new("market_info_v1.json"));

    let mut plan = AgentPlan::new(request.task_id.clone(), "incomplete");
    plan.push_step(navigation_step("https://www.baidu.com"));
    plan.push_step(observe_step());
    plan.push_step(click_step());
    plan.push_step(parse_step());

    let validator = PlanValidator::default();
    assert!(validator.validate(&plan, &request).is_err());
}

#[test]
fn validator_requires_observation_step_for_dom_parsers() {
    let mut request = AgentRequest::new(TaskId::new(), "Collect news");
    request
        .intent
        .required_outputs
        .push(agent_core::RequestedOutput::new("news_brief_v1.json"));

    let mut plan = AgentPlan::new(request.task_id.clone(), "missing-observe");
    plan.push_step(navigation_step("https://news.example.com"));
    plan.push_step(parse_step());
    plan.push_step(deliver_step());

    let validator = PlanValidator::default();
    assert!(validator.validate(&plan, &request).is_err());
}

#[test]
fn validator_accepts_github_api_pipeline_without_dom_observation() {
    let mut request = AgentRequest::new(TaskId::new(), "Collect GitHub repos");
    request
        .intent
        .required_outputs
        .push(agent_core::RequestedOutput::new("github_repos_v1.json"));

    let mut plan = AgentPlan::new(request.task_id.clone(), "github-only");
    plan.push_step(navigation_step("https://github.com/octocat"));
    plan.push_step(github_parse_step(json!({ "username": "octocat" })));
    plan.push_step(AgentPlanStep::new(
        "deliver-github",
        "Deliver repos",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.deliver.structured".to_string(),
                payload: json!({ "schema": "github_repos_v1" }),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    ));

    let validator = PlanValidator::default();
    assert!(validator.validate(&plan, &request).is_ok());
}

#[test]
fn validator_rejects_github_parse_without_username() {
    let mut request = AgentRequest::new(TaskId::new(), "Collect GitHub repos");
    request
        .intent
        .required_outputs
        .push(agent_core::RequestedOutput::new("github_repos_v1.json"));

    let mut plan = AgentPlan::new(request.task_id.clone(), "github-missing");
    plan.push_step(navigation_step("https://github.com/octocat"));
    plan.push_step(github_parse_step(json!({})));
    plan.push_step(AgentPlanStep::new(
        "deliver-github",
        "Deliver repos",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.deliver.structured".to_string(),
                payload: json!({ "schema": "github_repos_v1" }),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    ));

    let validator = PlanValidator::default();
    assert!(validator.validate(&plan, &request).is_err());
}

#[test]
fn validator_rejects_unknown_parser_tool() {
    let mut request = AgentRequest::new(TaskId::new(), "Collect LinkedIn profile");
    request
        .intent
        .required_outputs
        .push(agent_core::RequestedOutput::new("linkedin_profile_v1.json"));

    let mut plan = AgentPlan::new(request.task_id.clone(), "invalid");
    plan.push_step(navigation_step("https://www.linkedin.com/in/example"));
    plan.push_step(observe_step());
    plan.push_step(AgentPlanStep::new(
        "step-parse",
        "Parse profile",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.parse.linkedin-profile".to_string(),
                payload: json!({}),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    ));
    plan.push_step(deliver_step());

    let validator = PlanValidator::default();
    assert!(validator.validate(&plan, &request).is_err());
}
