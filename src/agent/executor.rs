use std::{sync::Arc, time::Duration};

use agent_core::{
    AgentContext, AgentLocator, AgentPlan, AgentPlanStep, AgentRequest, AgentScrollTarget,
    AgentTool, AgentToolKind, AgentValidation, AgentWaitCondition, WaitMode,
};
use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde_json::{json, Value};
use soulbrowser_core_types::{ActionId, ExecRoute, RoutePrefer, RoutingHint, TaskId, ToolCall};
use soulbrowser_scheduler::model::{
    CallOptions, DispatchOutput, DispatchRequest, DispatchTimeline, Priority, RetryOpt,
};
use soulbrowser_scheduler::Dispatcher;
use tracing::{debug, info, warn};

use crate::app_context::AppContext;

/// Execution options for running an agent plan.
#[derive(Clone, Debug)]
pub struct FlowExecutionOptions {
    /// Maximum number of retry attempts per tool (including the first try).
    pub max_retries: u8,
    /// Scheduler priority used for the dispatched tool calls.
    pub priority: Priority,
}

impl Default for FlowExecutionOptions {
    fn default() -> Self {
        Self {
            max_retries: 1,
            priority: Priority::Standard,
        }
    }
}

/// Status for each executed plan step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepExecutionStatus {
    Success,
    Failed,
}

/// Report describing an individual step execution.
#[derive(Clone, Debug)]
pub struct StepExecutionReport {
    pub step_id: String,
    pub title: String,
    pub status: StepExecutionStatus,
    pub attempts: u8,
    pub error: Option<String>,
    pub dispatches: Vec<DispatchRecord>,
}

/// Aggregate execution report for an entire plan.
#[derive(Clone, Debug)]
pub struct FlowExecutionReport {
    pub success: bool,
    pub steps: Vec<StepExecutionReport>,
}

/// Execute an agent plan using the application's scheduler and tool runtime.
pub async fn execute_plan(
    context: Arc<AppContext>,
    request: &AgentRequest,
    plan: &AgentPlan,
    options: FlowExecutionOptions,
) -> Result<FlowExecutionReport> {
    let dispatcher = context.scheduler_service();
    debug!(task = %request.task_id.0, "Executing plan via scheduler");

    // Ensure routing hint is derived from the agent context, if any.
    let routing_hint = build_routing_hint(request.context.as_ref());

    let mut reports = Vec::new();
    let mut all_success = true;

    for step in &plan.steps {
        let report = execute_step(
            dispatcher.as_ref(),
            request.task_id.clone(),
            step,
            &routing_hint,
            &options,
        )
        .await;

        if matches!(report.status, StepExecutionStatus::Failed) {
            all_success = false;
            warn!(step = %report.step_id, error = ?report.error, "Plan step failed");
            reports.push(report);
            break;
        }

        reports.push(report);
    }

    if all_success {
        info!(task = %request.task_id.0, steps = plan.steps.len(), "Agent plan executed successfully");
    }

    Ok(FlowExecutionReport {
        success: all_success,
        steps: reports,
    })
}

async fn execute_step<D: Dispatcher + ?Sized>(
    dispatcher: &D,
    task_id: TaskId,
    step: &AgentPlanStep,
    routing_hint: &Option<RoutingHint>,
    options: &FlowExecutionOptions,
) -> StepExecutionReport {
    let total_attempts = options.max_retries.max(1);
    let mut attempts = 0;
    let mut last_error: Option<String> = None;

    let specs = match build_dispatch_specs(step) {
        Ok(specs) => specs,
        Err(err) => {
            return StepExecutionReport {
                step_id: step.id.clone(),
                title: step.title.clone(),
                status: StepExecutionStatus::Failed,
                attempts: 0,
                error: Some(err.to_string()),
                dispatches: Vec::new(),
            }
        }
    };

    let mut last_dispatches: Vec<DispatchRecord> = Vec::new();

    for attempt in 0..total_attempts {
        attempts += 1;
        let mut failed = false;
        let mut step_dispatches: Vec<DispatchRecord> = Vec::new();

        for spec in &specs {
            let timeout = Duration::from_millis(spec.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
            let call_options = CallOptions {
                timeout,
                priority: options.priority,
                interruptible: true,
                retry: RetryOpt {
                    max: 0,
                    backoff: Duration::from_millis(200),
                },
            };

            let tool_call = ToolCall {
                call_id: Some(format!("{}::{}", step.id, spec.label)),
                task_id: Some(task_id.clone()),
                tool: spec.tool.clone(),
                payload: spec.payload.clone(),
            };

            let request = DispatchRequest {
                tool_call,
                options: call_options.clone(),
                routing_hint: routing_hint.clone(),
            };

            match dispatch_once(dispatcher, request).await {
                Ok((action_id, output)) => {
                    let (wait_ms, run_ms) = timeline_metrics(&output.timeline);
                    let (normalized_output, artifacts) =
                        normalize_dispatch_output(&spec.label, output.output.clone());
                    let error = output.error.map(|err| err.to_string());
                    if let Some(err) = error.clone() {
                        failed = true;
                        last_error = Some(err);
                    }
                    step_dispatches.push(DispatchRecord {
                        label: spec.label.clone(),
                        action_id: action_id.0.clone(),
                        route: output.route.clone(),
                        wait_ms,
                        run_ms,
                        output: normalized_output,
                        artifacts,
                        error,
                    });
                    if failed {
                        break;
                    }
                }
                Err(err) => {
                    failed = true;
                    last_error = Some(err.to_string());
                    break;
                }
            }
        }

        if !failed {
            return StepExecutionReport {
                step_id: step.id.clone(),
                title: step.title.clone(),
                status: StepExecutionStatus::Success,
                attempts,
                error: None,
                dispatches: step_dispatches,
            };
        }

        debug!(step = %step.id, attempt, "Retrying plan step after failure");
        last_dispatches = step_dispatches;
    }

    StepExecutionReport {
        step_id: step.id.clone(),
        title: step.title.clone(),
        status: StepExecutionStatus::Failed,
        attempts,
        error: last_error,
        dispatches: last_dispatches,
    }
}

async fn dispatch_once<D: Dispatcher + ?Sized>(
    dispatcher: &D,
    request: DispatchRequest,
) -> Result<(ActionId, DispatchOutput)> {
    let handle = dispatcher
        .submit(request)
        .await
        .map_err(|err| anyhow!("scheduler submit failed: {}", err))?;

    let action_id = handle.action_id.clone();

    let output = handle
        .receiver
        .await
        .map_err(|err| anyhow!("scheduler output channel closed: {}", err))?;

    Ok((action_id, output))
}

fn timeline_metrics(timeline: &DispatchTimeline) -> (u64, u64) {
    let wait_ms = timeline
        .started_at
        .map(|start| start.duration_since(timeline.enqueued_at).as_millis() as u64)
        .unwrap_or(0);
    let run_ms = match (timeline.started_at, timeline.finished_at) {
        (Some(start), Some(finish)) => finish.duration_since(start).as_millis() as u64,
        _ => 0,
    };
    (wait_ms, run_ms)
}

struct DispatchSpec {
    tool: String,
    payload: Value,
    timeout_ms: Option<u64>,
    label: String,
}

#[derive(Clone, Debug)]
pub struct DispatchRecord {
    pub label: String,
    pub action_id: String,
    pub route: ExecRoute,
    pub wait_ms: u64,
    pub run_ms: u64,
    pub output: Option<Value>,
    pub artifacts: Vec<RunArtifact>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RunArtifact {
    pub label: String,
    pub content_type: String,
    pub data_base64: String,
    pub byte_len: usize,
    pub filename: Option<String>,
}

fn normalize_dispatch_output(
    label: &str,
    output: Option<Value>,
) -> (Option<Value>, Vec<RunArtifact>) {
    let mut artifacts = Vec::new();
    let Some(mut value) = output else {
        return (None, artifacts);
    };

    if let Some(obj) = value.as_object_mut() {
        if let Some(bytes_value) = obj.remove("bytes") {
            if let Some(bytes) = extract_bytes(bytes_value) {
                let encoded = BASE64.encode(&bytes);
                let content_type = obj
                    .get("content_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let filename = obj
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                obj.insert("bytes_base64".to_string(), Value::String(encoded.clone()));
                obj.insert("byte_len".to_string(), Value::from(bytes.len() as u64));

                artifacts.push(RunArtifact {
                    label: label.to_string(),
                    content_type,
                    data_base64: encoded,
                    byte_len: bytes.len(),
                    filename,
                });
            }
        }
    }

    (Some(value), artifacts)
}

fn extract_bytes(value: Value) -> Option<Vec<u8>> {
    match value {
        Value::Array(items) => {
            let mut bytes = Vec::with_capacity(items.len());
            for item in items {
                let n = item.as_u64()?;
                bytes.push(n as u8);
            }
            Some(bytes)
        }
        Value::String(str_bytes) => BASE64.decode(str_bytes).ok(),
        _ => None,
    }
}

fn build_dispatch_specs(step: &AgentPlanStep) -> Result<Vec<DispatchSpec>> {
    let mut specs = Vec::new();
    specs.push(step_to_spec(step)?);

    for (idx, validation) in step.validations.iter().enumerate() {
        if let Some(spec) = validation_to_spec(step, validation, idx)? {
            specs.push(spec);
        }
    }

    Ok(specs)
}

fn step_to_spec(step: &AgentPlanStep) -> Result<DispatchSpec> {
    let (tool, mut payload) = tool_payload(&step.tool)?;
    if let Some(wait) = wait_mode_value(step.tool.wait) {
        payload
            .as_object_mut()
            .expect("tool payload must be object")
            .insert("wait_tier".to_string(), Value::String(wait.to_string()));
    }

    Ok(DispatchSpec {
        tool,
        payload,
        timeout_ms: step.tool.timeout_ms,
        label: "action".to_string(),
    })
}

fn validation_to_spec(
    step: &AgentPlanStep,
    validation: &AgentValidation,
    index: usize,
) -> Result<Option<DispatchSpec>> {
    match &validation.condition {
        AgentWaitCondition::ElementVisible(locator) => {
            let payload = json!({
                "target": {
                    "anchor": locator_to_json(locator),
                },
                "condition": {
                    "kind": "visible"
                }
            });
            Ok(Some(DispatchSpec {
                tool: "wait-for-element".to_string(),
                payload,
                timeout_ms: None,
                label: format!("validation-{}", index),
            }))
        }
        AgentWaitCondition::ElementHidden(locator) => {
            let payload = json!({
                "target": {
                    "anchor": locator_to_json(locator),
                },
                "condition": {
                    "kind": "hidden"
                }
            });
            Ok(Some(DispatchSpec {
                tool: "wait-for-element".to_string(),
                payload,
                timeout_ms: None,
                label: format!("validation-{}", index),
            }))
        }
        AgentWaitCondition::NetworkIdle(ms) => {
            let payload = json!({
                "expect": {
                    "net": { "quiet_ms": ms }
                }
            });
            Ok(Some(DispatchSpec {
                tool: "wait-for-condition".to_string(),
                payload,
                timeout_ms: Some(*ms),
                label: format!("validation-{}", index),
            }))
        }
        AgentWaitCondition::Duration(ms) => {
            let payload = json!({
                "expect": { "duration_ms": ms }
            });
            Ok(Some(DispatchSpec {
                tool: "wait-for-condition".to_string(),
                payload,
                timeout_ms: Some(*ms),
                label: format!("validation-{}", index),
            }))
        }
        AgentWaitCondition::UrlMatches(pattern) => {
            warn!(
                step = %step.id,
                pattern,
                "URL match validation not yet supported; skipping"
            );
            Ok(None)
        }
        AgentWaitCondition::TitleMatches(pattern) => {
            warn!(
                step = %step.id,
                pattern,
                "Title match validation not yet supported; skipping"
            );
            Ok(None)
        }
    }
}

fn tool_payload(tool: &AgentTool) -> Result<(String, Value)> {
    match &tool.kind {
        AgentToolKind::Navigate { url } => {
            Ok(("navigate-to-url".to_string(), json!({ "url": url })))
        }
        AgentToolKind::Click { locator } => Ok((
            "click".to_string(),
            json!({ "anchor": locator_to_json(locator) }),
        )),
        AgentToolKind::TypeText {
            locator,
            text,
            submit,
        } => Ok((
            "type-text".to_string(),
            json!({
                "anchor": locator_to_json(locator),
                "text": text,
                "submit": submit,
            }),
        )),
        AgentToolKind::Select {
            locator,
            value,
            method,
        } => Ok((
            "select-option".to_string(),
            json!({
                "anchor": locator_to_json(locator),
                "value": value,
                "match_kind": method.as_deref().unwrap_or("value")
            }),
        )),
        AgentToolKind::Scroll { target } => Ok((
            "scroll-page".to_string(),
            json!({
                "target": scroll_target_json(target),
                "behavior": "smooth",
            }),
        )),
        AgentToolKind::Wait { condition } => wait_tool_payload(condition),
        AgentToolKind::Custom { name, .. } => Err(anyhow!(
            "Custom tool '{}' is not supported for automated execution",
            name
        )),
    }
}

fn wait_tool_payload(condition: &AgentWaitCondition) -> Result<(String, Value)> {
    match condition {
        AgentWaitCondition::ElementVisible(locator) => Ok((
            "wait-for-element".to_string(),
            json!({
                "target": { "anchor": locator_to_json(locator) },
                "condition": { "kind": "visible" }
            }),
        )),
        AgentWaitCondition::ElementHidden(locator) => Ok((
            "wait-for-element".to_string(),
            json!({
                "target": { "anchor": locator_to_json(locator) },
                "condition": { "kind": "hidden" }
            }),
        )),
        AgentWaitCondition::NetworkIdle(ms) => Ok((
            "wait-for-condition".to_string(),
            json!({ "expect": { "net": { "quiet_ms": ms } } }),
        )),
        AgentWaitCondition::Duration(ms) => Ok((
            "wait-for-condition".to_string(),
            json!({ "expect": { "duration_ms": ms } }),
        )),
        AgentWaitCondition::UrlMatches(pattern) => Err(anyhow!(
            "Waiting for URL matches ('{}') is not supported yet",
            pattern
        )),
        AgentWaitCondition::TitleMatches(pattern) => Err(anyhow!(
            "Waiting for title matches ('{}') is not supported yet",
            pattern
        )),
    }
}

fn locator_to_json(locator: &AgentLocator) -> Value {
    match locator {
        AgentLocator::Css(selector) => Value::String(selector.clone()),
        AgentLocator::Aria { role, name } => json!({
            "role": role,
            "name": name,
        }),
        AgentLocator::Text { content, exact } => json!({
            "text": content,
            "exact": exact,
        }),
    }
}

fn scroll_target_json(target: &AgentScrollTarget) -> Value {
    match target {
        AgentScrollTarget::Top => json!({ "kind": "top" }),
        AgentScrollTarget::Bottom => json!({ "kind": "bottom" }),
        AgentScrollTarget::Selector(locator) => json!({
            "kind": "element",
            "anchor": locator_to_json(locator)
        }),
        AgentScrollTarget::Pixels(delta) => json!({
            "kind": "pixels",
            "value": delta
        }),
    }
}

fn wait_mode_value(mode: WaitMode) -> Option<&'static str> {
    match mode {
        WaitMode::None => None,
        WaitMode::DomReady => Some("domready"),
        WaitMode::Idle => Some("idle"),
    }
}

fn build_routing_hint(context: Option<&AgentContext>) -> Option<RoutingHint> {
    let ctx = context?;
    let mut hint = RoutingHint::default();
    hint.session = ctx.session.clone();
    hint.page = ctx.page.clone();
    hint.frame = ctx
        .metadata
        .get("frame_id")
        .and_then(|v| v.as_str())
        .map(|frame| soulbrowser_core_types::FrameId(frame.to_string()));
    hint.prefer = Some(RoutePrefer::Focused);

    if hint.session.is_some() || hint.page.is_some() || hint.frame.is_some() {
        Some(hint)
    } else {
        None
    }
}

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
