use agent_core::AgentError;
use agent_core::{
    AgentLocator, AgentPlan, AgentPlanMeta, AgentPlanStep, AgentRequest, AgentScrollTarget,
    AgentTool, AgentToolKind, AgentValidation, AgentWaitCondition, PlannerOutcome, WaitMode,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

use crate::agent::{ContextResolver, StageContext};

#[derive(Debug, Deserialize)]
pub struct LlmJsonPlan {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub rationale: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub vendor_context: serde_json::Value,
    pub steps: Vec<LlmJsonStep>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LlmJsonStep {
    pub title: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub evaluation: Option<String>,
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub next_goal: Option<String>,
    #[serde(default = "default_action")]
    pub action: String,
    #[serde(default)]
    pub locator: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub submit: Option<bool>,
    #[serde(default)]
    pub wait: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub validations: Vec<LlmJsonValidation>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LlmJsonValidation {
    pub description: Option<String>,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub argument: Option<String>,
}

fn default_action() -> String {
    "custom".to_string()
}

pub fn plan_from_json_payload(
    request: &AgentRequest,
    payload: LlmJsonPlan,
) -> Result<PlannerOutcome, AgentError> {
    if payload.steps.is_empty() {
        return Err(AgentError::invalid_request(
            "LLM response did not contain plan steps",
        ));
    }

    let mut plan = AgentPlan::new(request.task_id.clone(), payload.title.clone())
        .with_description(payload.description.clone());

    let stage_context = ContextResolver::new(request).build();

    for (index, step) in payload.steps.iter().enumerate() {
        let tool = to_agent_tool(step, &stage_context)?;
        let step_id = format!("llm-step-{}", index + 1);
        let mut agent_step =
            AgentPlanStep::new(step_id, step.title.clone(), tool).with_detail(step.detail.clone());
        agent_step.metadata.insert(
            "source".to_string(),
            json!({
                "action": step.action,
                "target": step.target,
                "locator": step.locator,
            }),
        );
        if let Some(state) = agent_state_metadata(step) {
            agent_step
                .metadata
                .insert("agent_state".to_string(), Value::Object(state));
        }
        if !step.validations.is_empty() {
            let validations = step
                .validations
                .iter()
                .filter_map(|v| to_validation(v).ok())
                .collect::<Vec<_>>();
            if !validations.is_empty() {
                agent_step.validations = validations;
            }
        }
        plan.push_step(agent_step);
    }

    let vendor_context: HashMap<String, serde_json::Value> = match payload.vendor_context {
        serde_json::Value::Object(map) => map.into_iter().collect(),
        _ => Default::default(),
    };

    plan.meta = AgentPlanMeta {
        rationale: payload.rationale.clone(),
        risk_assessment: payload.risks.clone(),
        vendor_context,
        overlays: Vec::new(),
    };

    let explanations = if plan.meta.rationale.is_empty() {
        vec![format!(
            "Plan generated via LLM for task {}",
            request.task_id.0
        )]
    } else {
        plan.meta.rationale.clone()
    };

    Ok(PlannerOutcome { plan, explanations })
}

fn agent_state_metadata(step: &LlmJsonStep) -> Option<Map<String, Value>> {
    let mut state = Map::new();
    if let Some(value) = clean_agent_state_text(&step.thinking) {
        state.insert("thinking".to_string(), Value::String(value));
    }
    if let Some(value) = clean_agent_state_text(&step.evaluation) {
        state.insert("evaluation".to_string(), Value::String(value));
    }
    if let Some(value) = clean_agent_state_text(&step.memory) {
        state.insert("memory".to_string(), Value::String(value));
    }
    if let Some(value) = clean_agent_state_text(&step.next_goal) {
        state.insert("next_goal".to_string(), Value::String(value));
    }
    if state.is_empty() {
        None
    } else {
        Some(state)
    }
}

fn clean_agent_state_text(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .map(|text| text.to_string())
}

fn to_agent_tool(step: &LlmJsonStep, context: &StageContext) -> Result<AgentTool, AgentError> {
    let action = step.action.trim().to_ascii_lowercase();
    let wait = step
        .wait
        .as_deref()
        .map(wait_mode_from_str)
        .unwrap_or(WaitMode::DomReady);

    let timeout_ms = step.timeout_ms;

    let kind = match action.as_str() {
        "navigate" | "nav" => AgentToolKind::Navigate {
            url: resolve_navigate_url(step, context)?,
        },
        "click" => AgentToolKind::Click {
            locator: parse_locator(step.locator.as_deref())?,
        },
        "type" | "type_text" | "type-text" | "input" => AgentToolKind::TypeText {
            locator: parse_locator(step.locator.as_deref())?,
            text: step
                .text
                .clone()
                .ok_or_else(|| AgentError::invalid_request("type_text step missing text"))?,
            submit: step.submit.unwrap_or(false),
        },
        "select" => AgentToolKind::Select {
            locator: parse_locator(step.locator.as_deref())?,
            value: step
                .value
                .clone()
                .ok_or_else(|| AgentError::invalid_request("select step missing value"))?,
            method: step.method.clone(),
        },
        "scroll" => AgentToolKind::Scroll {
            target: parse_scroll_target(step.target.as_deref())?,
        },
        "wait" => AgentToolKind::Wait {
            condition: parse_wait_condition(step)?,
        },
        "agent.evaluate" | "evaluate" => AgentToolKind::Custom {
            name: "agent.evaluate".to_string(),
            payload: json!({
                "title": step.title,
                "detail": step.detail,
            }),
        },
        _ => AgentToolKind::Custom {
            name: step.action.clone(),
            payload: json!({
                "title": step.title,
                "detail": step.detail,
                "locator": step.locator,
                "url": step.url,
                "text": step.text,
                "value": step.value,
            }),
        },
    };

    Ok(AgentTool {
        kind,
        wait,
        timeout_ms,
    })
}

fn resolve_navigate_url(step: &LlmJsonStep, context: &StageContext) -> Result<String, AgentError> {
    let explicit = step
        .url
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    if let Some(url) = explicit {
        return Ok(url);
    }
    if let Some(best) = context.best_known_url() {
        return Ok(best);
    }
    Ok(context.fallback_search_url())
}

fn parse_locator(raw: Option<&str>) -> Result<AgentLocator, AgentError> {
    let locator = raw
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AgentError::invalid_request("step missing locator"))?;

    if let Some(rest) = locator.strip_prefix("css=") {
        return Ok(AgentLocator::Css(rest.trim().to_string()));
    }
    if let Some(rest) = locator.strip_prefix("text=") {
        return Ok(AgentLocator::Text {
            content: rest.trim().to_string(),
            exact: false,
        });
    }
    if let Some(rest) = locator.strip_prefix("aria:") {
        let mut parts = rest.splitn(2, '=');
        let role = parts
            .next()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "button".to_string());
        let name = parts
            .next()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        return Ok(AgentLocator::Aria { role, name });
    }

    Ok(AgentLocator::Css(locator.to_string()))
}

fn parse_scroll_target(raw: Option<&str>) -> Result<AgentScrollTarget, AgentError> {
    match raw.map(|s| s.trim().to_ascii_lowercase()) {
        Some(ref value) if value == "top" => Ok(AgentScrollTarget::Top),
        Some(ref value) if value == "bottom" => Ok(AgentScrollTarget::Bottom),
        Some(value) if value.starts_with("css=") || value.starts_with("text=") => {
            let locator = parse_locator(Some(&value))?;
            Ok(AgentScrollTarget::Selector(locator))
        }
        Some(value) if value.starts_with("pixels=") => {
            let amount = value
                .splitn(2, '=')
                .nth(1)
                .and_then(|n| n.parse::<i32>().ok())
                .ok_or_else(|| AgentError::invalid_request("scroll target pixels invalid"))?;
            Ok(AgentScrollTarget::Pixels(amount))
        }
        Some(value) if value.starts_with("pixels:") => {
            let amount = value
                .splitn(2, ':')
                .nth(1)
                .and_then(|n| n.trim().parse::<i32>().ok())
                .ok_or_else(|| AgentError::invalid_request("scroll target pixels invalid"))?;
            Ok(AgentScrollTarget::Pixels(amount))
        }
        None => Err(AgentError::invalid_request("scroll step missing target")),
        Some(other) => {
            let locator = parse_locator(Some(&other))?;
            Ok(AgentScrollTarget::Selector(locator))
        }
    }
}

fn parse_wait_condition(step: &LlmJsonStep) -> Result<AgentWaitCondition, AgentError> {
    if let Some(target) = step.target.as_ref() {
        let target = target.trim();
        if let Some(url) = target.strip_prefix("url=") {
            return Ok(AgentWaitCondition::UrlMatches(url.trim().to_string()));
        }
        if let Some(url) = target
            .strip_prefix("url_equals=")
            .or_else(|| target.strip_prefix("url_exact="))
        {
            return Ok(AgentWaitCondition::UrlEquals(url.trim().to_string()));
        }
        if let Some(selector) = target.strip_prefix("css=") {
            return Ok(AgentWaitCondition::ElementVisible(AgentLocator::Css(
                selector.trim().to_string(),
            )));
        }
        if let Some(selector) = target.strip_prefix("text=") {
            return Ok(AgentWaitCondition::ElementVisible(AgentLocator::Text {
                content: selector.trim().to_string(),
                exact: false,
            }));
        }
    }

    if let Some(value) = step.value.as_ref() {
        if let Ok(duration_ms) = value.trim().parse::<u64>() {
            return Ok(AgentWaitCondition::Duration(duration_ms));
        }
    }

    Ok(AgentWaitCondition::NetworkIdle(1_000))
}

fn wait_mode_from_str(value: &str) -> WaitMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => WaitMode::None,
        "idle" | "network_idle" => WaitMode::Idle,
        _ => WaitMode::DomReady,
    }
}

fn to_validation(raw: &LlmJsonValidation) -> Result<AgentValidation, AgentError> {
    let description = raw
        .description
        .clone()
        .unwrap_or_else(|| "LLM validation".to_string());
    let condition = match raw.kind.to_ascii_lowercase().as_str() {
        "url_matches" | "url" => AgentWaitCondition::UrlMatches(
            raw.argument
                .clone()
                .ok_or_else(|| AgentError::invalid_request("validation missing argument"))?,
        ),
        "url_equals" | "url_exact" => AgentWaitCondition::UrlEquals(
            raw.argument
                .clone()
                .ok_or_else(|| AgentError::invalid_request("validation missing argument"))?,
        ),
        "element_visible" | "visible" => {
            AgentWaitCondition::ElementVisible(parse_locator(raw.argument.as_deref())?)
        }
        "element_hidden" | "hidden" => {
            AgentWaitCondition::ElementHidden(parse_locator(raw.argument.as_deref())?)
        }
        _ => AgentWaitCondition::Duration(500),
    };

    Ok(AgentValidation {
        description,
        condition,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{AgentContext, ConversationRole, ConversationTurn};
    use soulbrowser_core_types::TaskId;
    use std::collections::HashMap;

    fn base_request(goal: &str) -> AgentRequest {
        let mut req = AgentRequest::new(TaskId::new(), goal);
        req.push_turn(ConversationTurn::new(
            ConversationRole::User,
            goal.to_string(),
        ));
        req
    }

    #[test]
    fn parses_basic_plan() {
        let request = base_request("Open example.com and click Sign In");
        let payload = LlmJsonPlan {
            title: "Sample plan".into(),
            description: "Do the task".into(),
            rationale: vec!["Step-by-step".into()],
            risks: vec!["Login might fail".into()],
            vendor_context: json!({"model": "mock"}),
            steps: vec![
                LlmJsonStep {
                    title: "Navigate".into(),
                    detail: "Open the landing page".into(),
                    action: "navigate".into(),
                    url: Some("https://example.com".into()),
                    ..Default::default()
                },
                LlmJsonStep {
                    title: "Click sign in".into(),
                    detail: "Open auth modal".into(),
                    action: "click".into(),
                    locator: Some("css=button.sign-in".into()),
                    validations: vec![LlmJsonValidation {
                        description: Some("Auth modal visible".into()),
                        kind: "element_visible".into(),
                        argument: Some("css=.auth-modal".into()),
                    }],
                    ..Default::default()
                },
            ],
        };

        let outcome = plan_from_json_payload(&request, payload).expect("plan parsed");
        assert_eq!(outcome.plan.steps.len(), 2);
        assert_eq!(outcome.plan.title, "Sample plan");
        assert_eq!(outcome.plan.meta.rationale.len(), 1);
        assert_eq!(
            outcome.plan.meta.vendor_context.get("model").unwrap(),
            "mock"
        );
        assert_eq!(outcome.plan.steps[1].validations.len(), 1);
    }

    #[test]
    fn invalid_step_without_locator_fails() {
        let request = base_request("Click something");
        let payload = LlmJsonPlan {
            title: "Bad plan".into(),
            description: String::new(),
            rationale: vec![],
            risks: vec![],
            vendor_context: serde_json::Value::Null,
            steps: vec![LlmJsonStep {
                title: "Click".into(),
                action: "click".into(),
                ..Default::default()
            }],
        };

        let err = plan_from_json_payload(&request, payload).unwrap_err();
        assert!(format!("{err}").contains("locator"));
    }

    #[test]
    fn navigate_step_without_url_uses_context() {
        let mut request = base_request("Open current page");
        request.context = Some(AgentContext {
            session: None,
            page: None,
            current_url: Some("https://cached.example".into()),
            preferences: HashMap::new(),
            memory_hints: Vec::new(),
            metadata: HashMap::new(),
        });

        let payload = LlmJsonPlan {
            title: "Weather".into(),
            description: String::new(),
            rationale: vec![],
            risks: vec![],
            vendor_context: serde_json::Value::Null,
            steps: vec![LlmJsonStep {
                title: "Go".into(),
                action: "navigate".into(),
                ..Default::default()
            }],
        };

        let outcome = plan_from_json_payload(&request, payload).expect("plan parsed");
        match &outcome.plan.steps[0].tool.kind {
            AgentToolKind::Navigate { url } => {
                assert_eq!(url, "https://cached.example");
            }
            other => panic!("unexpected tool: {:?}", other),
        }
    }
}
