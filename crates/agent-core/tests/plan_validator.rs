use agent_core::{
    AgentIntentKind, AgentPlan, AgentPlanStep, AgentRequest, AgentTool, AgentToolKind,
    AgentValidation, AgentWaitCondition, PlanValidator, WaitMode,
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
    let mut step = AgentPlanStep::new(
        "step-3",
        "Click",
        AgentTool {
            kind: AgentToolKind::Click {
                locator: agent_core::AgentLocator::Css("button".to_string()),
            },
            wait: WaitMode::DomReady,
            timeout_ms: None,
        },
    );
    step.validations.push(AgentValidation {
        description: "Wait for navigation".to_string(),
        condition: AgentWaitCondition::UrlMatches("https://example.com/results".to_string()),
    });
    step
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
                payload: json!({
                    "schema": "market_info_v1",
                    "artifact_label": "market_info",
                    "filename": "market_info_v1.json",
                    "source_step_id": "step-4"
                }),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    )
}

fn weather_parse_step() -> AgentPlanStep {
    AgentPlanStep::new(
        "weather-parse",
        "Parse weather",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.parse.weather".to_string(),
                payload: json!({
                    "source_step_id": "stage-observe",
                }),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    )
}

fn weather_deliver_step() -> AgentPlanStep {
    AgentPlanStep::new(
        "deliver-weather",
        "Deliver weather",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.deliver.structured".to_string(),
                payload: json!({
                    "schema": "weather_report_v1",
                    "artifact_label": "structured.weather_report_v1",
                    "filename": "weather_report_v1.json",
                    "source_step_id": "weather-parse"
                }),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    )
}

fn bare_click_step() -> AgentPlanStep {
    AgentPlanStep::new(
        "step-click",
        "Click result link",
        AgentTool {
            kind: AgentToolKind::Click {
                locator: agent_core::AgentLocator::Css("a.result".to_string()),
            },
            wait: WaitMode::DomReady,
            timeout_ms: None,
        },
    )
}

fn note_step(message: &str) -> AgentPlanStep {
    AgentPlanStep::new(
        "step-note",
        "Note",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "agent.note".to_string(),
                payload: json!({ "message": message }),
            },
            wait: WaitMode::None,
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
fn strict_validator_rejects_wrong_target_site() {
    let mut request = AgentRequest::new(TaskId::new(), "Check quotes");
    request.intent.target_sites = vec!["baidu".to_string()];
    let mut plan = AgentPlan::new(request.task_id.clone(), "test");
    plan.push_step(navigation_step("https://www.google.com"));

    let relaxed = PlanValidator::default();
    assert!(relaxed.validate(&plan, &request).is_ok());
    let strict = PlanValidator::strict();
    assert!(strict.validate(&plan, &request).is_err());
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
fn strict_validator_rejects_missing_deliver_step() {
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

    let relaxed = PlanValidator::default();
    assert!(relaxed.validate(&plan, &request).is_ok());
    let strict = PlanValidator::strict();
    assert!(strict.validate(&plan, &request).is_err());
}

#[test]
fn strict_validator_requires_observation_step_for_dom_parsers() {
    let mut request = AgentRequest::new(TaskId::new(), "Collect news");
    request
        .intent
        .required_outputs
        .push(agent_core::RequestedOutput::new("news_brief_v1.json"));

    let mut plan = AgentPlan::new(request.task_id.clone(), "missing-observe");
    plan.push_step(navigation_step("https://news.example.com"));
    plan.push_step(parse_step());
    plan.push_step(deliver_step());

    let relaxed = PlanValidator::default();
    assert!(relaxed.validate(&plan, &request).is_ok());
    let strict = PlanValidator::strict();
    assert!(strict.validate(&plan, &request).is_err());
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
                payload: json!({
                    "schema": "github_repos_v1",
                    "artifact_label": "github_repos",
                    "filename": "github_repos_v1.json",
                    "source_step_id": "step-github"
                }),
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
                payload: json!({
                    "schema": "github_repos_v1",
                    "artifact_label": "github_repos",
                    "filename": "github_repos_v1.json",
                    "source_step_id": "step-github"
                }),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    ));

    let validator = PlanValidator::default();
    assert!(validator.validate(&plan, &request).is_err());
}

#[test]
fn strict_validator_rejects_unknown_parser_tool() {
    let mut request = AgentRequest::new(TaskId::new(), "Collect LinkedIn profile");
    request
        .intent
        .required_outputs
        .push(agent_core::RequestedOutput::new("linkedin_profile_v1.json"));

    let mut plan = AgentPlan::new(request.task_id.clone(), "invalid");
    plan.push_step(navigation_step("https://www.linkedin.com/in/example"));
    plan.push_step(observe_step());
    let parse_id = "step-parse";
    plan.push_step(AgentPlanStep::new(
        parse_id,
        "Parse profile",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "plugin.linkedin.unknown".to_string(),
                payload: json!({}),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    ));
    plan.push_step(AgentPlanStep::new(
        "deliver-linkedin",
        "Deliver profile",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.deliver.structured".to_string(),
                payload: json!({
                    "schema": "linkedin_profile_v1",
                    "artifact_label": "linkedin_profile_v1",
                    "filename": "linkedin_profile_v1.json",
                    "source_step_id": parse_id,
                }),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    ));

    let relaxed = PlanValidator::default();
    assert!(relaxed.validate(&plan, &request).is_ok());
    let strict = PlanValidator::strict();
    assert!(strict.validate(&plan, &request).is_err());
}

#[test]
fn strict_informational_intents_require_full_pipeline() {
    let mut request = AgentRequest::new(TaskId::new(), "查询天气");
    request.intent.intent_kind = AgentIntentKind::Informational;

    let mut plan = AgentPlan::new(request.task_id.clone(), "incomplete-info");
    plan.push_step(navigation_step("https://weather.example.com"));
    plan.push_step(observe_step());
    plan.push_step(click_step());

    let relaxed = PlanValidator::default();
    assert!(relaxed.validate(&plan, &request).is_ok());
    let strict = PlanValidator::strict();
    assert!(strict.validate(&plan, &request).is_err());

    plan.push_step(weather_parse_step());
    plan.push_step(weather_deliver_step());
    assert!(strict.validate(&plan, &request).is_ok());
}

#[test]
fn strict_keyword_intents_need_user_result() {
    let mut request = AgentRequest::new(TaskId::new(), "查看最新报告");
    request.intent.intent_kind = AgentIntentKind::Operational;

    let mut plan = AgentPlan::new(request.task_id.clone(), "missing-result");
    plan.push_step(navigation_step("https://weather.example.com"));
    plan.push_step(observe_step());

    let relaxed = PlanValidator::default();
    assert!(relaxed.validate(&plan, &request).is_ok());
    let strict = PlanValidator::strict();
    assert!(strict.validate(&plan, &request).is_err());

    plan.push_step(note_step("记录结果摘要"));
    assert!(strict.validate(&plan, &request).is_ok());
}

#[test]
fn strict_weather_keywords_require_weather_parser() {
    let mut request = AgentRequest::new(TaskId::new(), "查询北京天气");
    request.intent.intent_kind = AgentIntentKind::Informational;

    let mut incomplete = AgentPlan::new(request.task_id.clone(), "weather-missing");
    incomplete.push_step(navigation_step("https://weather.example.com"));
    incomplete.push_step(observe_step());
    incomplete.push_step(click_step());
    incomplete.push_step(note_step("记录天气摘要"));

    let relaxed = PlanValidator::default();
    assert!(relaxed.validate(&incomplete, &request).is_ok());
    let strict = PlanValidator::strict();
    assert!(strict.validate(&incomplete, &request).is_err());

    let mut complete = AgentPlan::new(request.task_id.clone(), "weather-complete");
    complete.push_step(navigation_step("https://weather.example.com"));
    complete.push_step(observe_step());
    complete.push_step(click_step());
    complete.push_step(weather_parse_step());
    complete.push_step(weather_deliver_step());

    assert!(strict.validate(&complete, &request).is_ok());
}

#[test]
fn click_steps_require_wait_validations() {
    let request = AgentRequest::new(TaskId::new(), "测试点击");

    let mut plan = AgentPlan::new(request.task_id.clone(), "missing-click-validation");
    plan.push_step(bare_click_step());

    let validator = PlanValidator::default();
    assert!(validator.validate(&plan, &request).is_err());

    let mut fixed_click = bare_click_step();
    fixed_click.validations.push(AgentValidation {
        description: "Wait for results page".to_string(),
        condition: AgentWaitCondition::UrlMatches("https://example.com/result".to_string()),
    });
    let mut repaired_plan = AgentPlan::new(request.task_id.clone(), "click-fixed");
    repaired_plan.push_step(fixed_click);

    assert!(validator.validate(&repaired_plan, &request).is_ok());
}

#[test]
fn validator_rejects_deliver_missing_source_step_field() {
    let mut request = AgentRequest::new(TaskId::new(), "Collect markets");
    request
        .intent
        .required_outputs
        .push(agent_core::RequestedOutput::new("market_info_v1.json"));

    let mut plan = AgentPlan::new(request.task_id.clone(), "missing-source-field");
    plan.push_step(navigation_step("https://www.baidu.com"));
    plan.push_step(observe_step());
    plan.push_step(parse_step());
    plan.push_step(AgentPlanStep::new(
        "invalid-deliver",
        "Deliver without source",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.deliver.structured".to_string(),
                payload: json!({
                    "schema": "market_info_v1",
                    "artifact_label": "market_info",
                    "filename": "market_info_v1.json"
                }),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    ));

    let validator = PlanValidator::default();
    assert!(validator.validate(&plan, &request).is_err());
}

#[test]
fn validator_rejects_deliver_source_that_is_not_parse_step() {
    let mut request = AgentRequest::new(TaskId::new(), "Collect markets");
    request
        .intent
        .required_outputs
        .push(agent_core::RequestedOutput::new("market_info_v1.json"));

    let mut plan = AgentPlan::new(request.task_id.clone(), "deliver-non-parse-source");
    plan.push_step(navigation_step("https://www.baidu.com"));
    plan.push_step(observe_step());
    plan.push_step(parse_step());
    plan.push_step(AgentPlanStep::new(
        "invalid-deliver",
        "Deliver referencing navigate",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.deliver.structured".to_string(),
                payload: json!({
                    "schema": "market_info_v1",
                    "artifact_label": "market_info",
                    "filename": "market_info_v1.json",
                    "source_step_id": "step-1"
                }),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    ));

    let validator = PlanValidator::default();
    assert!(validator.validate(&plan, &request).is_err());
}

#[test]
fn validator_rejects_deliver_source_that_occurs_later() {
    let mut request = AgentRequest::new(TaskId::new(), "Collect markets");
    request
        .intent
        .required_outputs
        .push(agent_core::RequestedOutput::new("market_info_v1.json"));

    let mut plan = AgentPlan::new(request.task_id.clone(), "deliver-future-source");
    plan.push_step(navigation_step("https://www.baidu.com"));
    plan.push_step(observe_step());
    plan.push_step(parse_step());
    plan.push_step(AgentPlanStep::new(
        "deliver-mid",
        "Deliver referencing future parse",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.deliver.structured".to_string(),
                payload: json!({
                    "schema": "market_info_v1",
                    "artifact_label": "market_info",
                    "filename": "market_info_v1.json",
                    "source_step_id": "late-parse"
                }),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    ));
    plan.push_step(AgentPlanStep::new(
        "late-parse",
        "Late parse",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "data.parse.market_info".to_string(),
                payload: json!({}),
            },
            wait: WaitMode::Idle,
            timeout_ms: None,
        },
    ));

    let validator = PlanValidator::default();
    assert!(validator.validate(&plan, &request).is_err());
}
