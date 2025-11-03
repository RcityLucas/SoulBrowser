use crate::errors::AgentError;
use crate::plan::{
    AgentLocator, AgentPlan, AgentPlanStep, AgentScrollTarget, AgentTool, AgentToolKind,
    AgentValidation, AgentWaitCondition, WaitMode,
};
use action_flow::types::{ActionType, FailureStrategy, Flow, FlowNode};
use action_gate::{
    Condition, DomCondition, ExpectSpec, NetCondition, TitleCondition, UrlCondition,
};
use action_primitives::{
    AnchorDescriptor, ScrollBehavior, ScrollTarget, SelectMethod, WaitCondition, WaitTier,
};
use serde_json::{Map as JsonMap, Value};
use std::collections::HashMap;

/// Options controlling conversion from AgentPlan into Flow structures.
#[derive(Debug, Clone)]
pub struct PlanToFlowOptions {
    /// Flow-level timeout in milliseconds.
    pub flow_timeout_ms: u64,
    /// Default timeout for wait actions when none specified on the step.
    pub default_wait_timeout_ms: u64,
    /// Default timeout for validation specs when not explicitly provided.
    pub default_validation_timeout_ms: u64,
}

impl Default for PlanToFlowOptions {
    fn default() -> Self {
        Self {
            flow_timeout_ms: 300_000,
            default_wait_timeout_ms: 15_000,
            default_validation_timeout_ms: 10_000,
        }
    }
}

/// Result produced by `plan_to_flow`, including derived metadata.
#[derive(Debug)]
pub struct PlanToFlowResult {
    pub flow: Flow,
    pub step_count: usize,
    pub validation_count: usize,
}

/// Convert an `AgentPlan` into an `action_flow::Flow`.
pub fn plan_to_flow(
    plan: &AgentPlan,
    opts: PlanToFlowOptions,
) -> Result<PlanToFlowResult, AgentError> {
    if plan.steps.is_empty() {
        return Err(AgentError::invalid_request(
            "plan must contain at least one step",
        ));
    }

    let mut step_nodes = Vec::with_capacity(plan.steps.len());
    let mut validation_count = 0usize;

    for step in &plan.steps {
        let node = convert_step(step, &opts)?;
        validation_count += step.validations.len();
        step_nodes.push(node);
    }

    let root = if step_nodes.len() == 1 {
        step_nodes.remove(0)
    } else {
        FlowNode::Sequence { steps: step_nodes }
    };

    let mut flow = Flow::new(plan.task_id.0.clone(), plan.title.clone(), root)
        .with_description(plan.description.clone())
        .with_timeout(opts.flow_timeout_ms)
        .with_default_strategy(FailureStrategy::Abort);

    if !plan.meta.rationale.is_empty() {
        let rationale = Value::Array(
            plan.meta
                .rationale
                .iter()
                .map(|s| Value::String(s.clone()))
                .collect(),
        );
        flow = flow.with_metadata("plan_rationale".to_string(), rationale);
    }

    if !plan.meta.risk_assessment.is_empty() {
        let risks = Value::Array(
            plan.meta
                .risk_assessment
                .iter()
                .map(|s| Value::String(s.clone()))
                .collect(),
        );
        flow = flow.with_metadata("risk_assessment".to_string(), risks);
    }

    if !plan.meta.vendor_context.is_empty() {
        let mut obj = JsonMap::new();
        for (key, value) in &plan.meta.vendor_context {
            obj.insert(key.clone(), value.clone());
        }
        flow = flow.with_metadata("planner_context".to_string(), Value::Object(obj));
    }

    Ok(PlanToFlowResult {
        flow,
        step_count: plan.steps.len(),
        validation_count,
    })
}

fn convert_step(step: &AgentPlanStep, opts: &PlanToFlowOptions) -> Result<FlowNode, AgentError> {
    let action = convert_tool(&step.tool, opts)?;
    let expect = build_expectations(&step.validations, opts.default_validation_timeout_ms)?;

    Ok(FlowNode::Action {
        id: step.id.clone(),
        action,
        expect,
        failure_strategy: None,
    })
}

fn convert_tool(tool: &AgentTool, opts: &PlanToFlowOptions) -> Result<ActionType, AgentError> {
    match &tool.kind {
        AgentToolKind::Navigate { url } => {
            if url.is_empty() {
                return Err(AgentError::unsupported(
                    "navigate tool requires non-empty url",
                ));
            }
            Ok(ActionType::Navigate {
                url: url.clone(),
                wait_tier: map_wait_mode(tool.wait),
            })
        }
        AgentToolKind::Click { locator } => Ok(ActionType::Click {
            anchor: to_anchor(locator)?,
            wait_tier: map_wait_mode(tool.wait),
        }),
        AgentToolKind::TypeText {
            locator,
            text,
            submit,
        } => Ok(ActionType::TypeText {
            anchor: to_anchor(locator)?,
            text: text.clone(),
            submit: *submit,
            wait_tier: map_wait_mode(tool.wait),
        }),
        AgentToolKind::Select {
            locator,
            value,
            method,
        } => {
            let wait = match tool.wait {
                WaitMode::None => None,
                other => Some(map_wait_mode(other)),
            };
            Ok(ActionType::Select {
                anchor: to_anchor(locator)?,
                option: value.clone(),
                method: method.as_ref().map(|m| match m.as_str() {
                    "text" | "Text" => SelectMethod::Text,
                    "index" | "Index" => SelectMethod::Index,
                    _ => SelectMethod::Value,
                }),
                wait_tier: wait,
            })
        }
        AgentToolKind::Scroll { target } => Ok(ActionType::Scroll {
            target: to_scroll_target(target)?,
            behavior: ScrollBehavior::Smooth,
            wait_tier: map_wait_mode(tool.wait),
        }),
        AgentToolKind::Wait { condition } => Ok(ActionType::Wait {
            condition: to_wait_condition(condition)?,
            timeout_ms: tool.timeout_ms.unwrap_or(opts.default_wait_timeout_ms),
        }),
        AgentToolKind::Custom { name, payload } => Ok(ActionType::Custom {
            action_type: name.clone(),
            parameters: payload_to_map(payload),
        }),
    }
}

fn to_anchor(locator: &AgentLocator) -> Result<AnchorDescriptor, AgentError> {
    match locator {
        AgentLocator::Css(selector) => {
            if selector.trim().is_empty() {
                Err(AgentError::unsupported("css locator cannot be empty"))
            } else {
                Ok(AnchorDescriptor::Css(selector.clone()))
            }
        }
        AgentLocator::Aria { role, name } => {
            if role.trim().is_empty() || name.trim().is_empty() {
                Err(AgentError::unsupported(
                    "aria locator requires role and name",
                ))
            } else {
                Ok(AnchorDescriptor::Aria {
                    role: role.clone(),
                    name: name.clone(),
                })
            }
        }
        AgentLocator::Text { content, exact } => {
            if content.trim().is_empty() {
                Err(AgentError::unsupported("text locator cannot be empty"))
            } else {
                Ok(AnchorDescriptor::Text {
                    content: content.clone(),
                    exact: *exact,
                })
            }
        }
    }
}

fn map_wait_mode(mode: WaitMode) -> WaitTier {
    match mode {
        WaitMode::None => WaitTier::None,
        WaitMode::DomReady => WaitTier::DomReady,
        WaitMode::Idle => WaitTier::Idle,
    }
}

fn to_wait_condition(condition: &AgentWaitCondition) -> Result<WaitCondition, AgentError> {
    match condition {
        AgentWaitCondition::ElementVisible(locator) => {
            Ok(WaitCondition::ElementVisible(to_anchor(locator)?))
        }
        AgentWaitCondition::ElementHidden(locator) => {
            Ok(WaitCondition::ElementHidden(to_anchor(locator)?))
        }
        AgentWaitCondition::UrlMatches(pattern) => Ok(WaitCondition::UrlMatches(pattern.clone())),
        AgentWaitCondition::TitleMatches(pattern) => {
            Ok(WaitCondition::TitleMatches(pattern.clone()))
        }
        AgentWaitCondition::NetworkIdle(ms) => Ok(WaitCondition::NetworkIdle(*ms)),
        AgentWaitCondition::Duration(ms) => Ok(WaitCondition::Duration(*ms)),
    }
}

fn to_scroll_target(target: &AgentScrollTarget) -> Result<ScrollTarget, AgentError> {
    match target {
        AgentScrollTarget::Top => Ok(ScrollTarget::Top),
        AgentScrollTarget::Bottom => Ok(ScrollTarget::Bottom),
        AgentScrollTarget::Selector(locator) => Ok(ScrollTarget::Element(to_anchor(locator)?)),
        AgentScrollTarget::Pixels(delta) => Ok(ScrollTarget::Pixels(*delta)),
    }
}

fn build_expectations(
    validations: &[AgentValidation],
    default_timeout_ms: u64,
) -> Result<Option<ExpectSpec>, AgentError> {
    if validations.is_empty() {
        return Ok(None);
    }

    let mut spec = ExpectSpec::new().with_timeout(default_timeout_ms);
    for validation in validations {
        let condition = to_condition(&validation.condition)?;
        spec = spec.with_all(condition);
    }

    if spec.has_conditions() {
        Ok(Some(spec))
    } else {
        Ok(None)
    }
}

fn to_condition(condition: &AgentWaitCondition) -> Result<Condition, AgentError> {
    match condition {
        AgentWaitCondition::ElementVisible(locator) => Ok(Condition::Dom(
            DomCondition::ElementVisible(to_anchor(locator)?),
        )),
        AgentWaitCondition::ElementHidden(locator) => Ok(Condition::Dom(
            DomCondition::ElementHidden(to_anchor(locator)?),
        )),
        AgentWaitCondition::UrlMatches(pattern) => {
            Ok(Condition::Url(UrlCondition::Matches(pattern.clone())))
        }
        AgentWaitCondition::TitleMatches(pattern) => {
            Ok(Condition::Title(TitleCondition::Matches(pattern.clone())))
        }
        AgentWaitCondition::NetworkIdle(ms) => Ok(Condition::Net(NetCondition::NetworkIdle(*ms))),
        AgentWaitCondition::Duration(ms) => {
            // Duration-based validation is mapped to a network idle placeholder with zero delta,
            // ensuring validation waits for the specified delay before succeeding.
            Ok(Condition::Net(NetCondition::NetworkIdle(*ms)))
        }
    }
}

fn payload_to_map(payload: &serde_json::Value) -> HashMap<String, Value> {
    match payload {
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        other => {
            let mut map = HashMap::new();
            map.insert("value".to_string(), other.clone());
            map
        }
    }
}
