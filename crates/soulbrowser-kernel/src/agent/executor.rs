use std::{collections::HashMap, env, fmt, sync::Arc, time::Duration};

use agent_core::{
    requires_weather_pipeline, weather_query_text, weather_search_url, AgentContext, AgentLocator,
    AgentPlan, AgentPlanStep, AgentRequest, AgentScrollTarget, AgentTool, AgentToolKind,
    AgentValidation, AgentWaitCondition, WaitMode,
};
use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use cdp_adapter::AdapterMode;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use soulbrowser_core_types::{
    ActionId, ExecRoute, FrameId, PageId, RoutePrefer, RoutingHint, SessionId, TaskId, ToolCall,
};
use soulbrowser_registry::{Registry, RegistryImpl};
use soulbrowser_scheduler::model::{
    CallOptions, DispatchOutput, DispatchRequest, DispatchTimeline, Priority, RetryOpt,
};
use soulbrowser_scheduler::Dispatcher;
use tracing::{debug, info, warn};
use url::Url;

use crate::{
    agent::{EXPECTED_URL_METADATA_KEY, OBSERVATION_CANONICAL},
    app_context::AppContext,
    block_detect::detect_block_reason,
    metrics,
    task_status::TaskStatusHandle,
};

const WEATHER_GUARDRAIL_MESSAGE: &str =
    "weather pipeline is still rendering Baidu home; search results never loaded";

static PREVIEW_CAPTURE_ENABLED: Lazy<bool> =
    Lazy::new(|| match env::var("SOULBROWSER_LIVE_PREVIEW") {
        Ok(value) => matches!(value.trim(), "1" | "true" | "TRUE" | "True"),
        Err(_) => true,
    });

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
    pub tool_kind: String,
    pub status: StepExecutionStatus,
    pub attempts: u8,
    pub error: Option<String>,
    pub dispatches: Vec<DispatchRecord>,
    pub total_wait_ms: u64,
    pub total_run_ms: u64,
}

/// Aggregate execution report for an entire plan.
#[derive(Clone, Debug)]
pub struct FlowExecutionReport {
    pub success: bool,
    pub steps: Vec<StepExecutionReport>,
    pub user_results: Vec<UserResult>,
    pub missing_user_result: bool,
}

#[derive(Clone, Debug)]
pub enum UserResultKind {
    Note,
    Structured,
    Artifact,
}

impl UserResultKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserResultKind::Note => "note",
            UserResultKind::Structured => "structured",
            UserResultKind::Artifact => "artifact",
        }
    }
}

#[derive(Clone, Debug)]
pub struct UserResult {
    pub step_id: String,
    pub step_title: String,
    pub kind: UserResultKind,
    pub schema: Option<String>,
    pub content: Value,
    pub artifact_path: Option<String>,
}

/// Execute an agent plan using the application's scheduler and tool runtime.
pub async fn execute_plan(
    context: Arc<AppContext>,
    request: &AgentRequest,
    plan: &AgentPlan,
    options: FlowExecutionOptions,
    status_handle: Option<&TaskStatusHandle>,
) -> Result<FlowExecutionReport> {
    if plan_requires_dom_actions(plan) {
        if let Some(AdapterMode::Stub) = context.tool_manager().adapter_mode() {
            return Err(anyhow!(
                "Plan requires DOM tools (click/type/select/scroll) but the CDP adapter is running in stub mode. Install Chrome/Chromium and set SOULBROWSER_USE_REAL_CHROME=1 with SOULBROWSER_CHROME=/path/to/chrome, or launch with --chrome-path/--ws-url before executing."
            ));
        }
    }

    let dispatcher = context.scheduler_service();
    debug!(task = %request.task_id.0, "Executing plan via scheduler");

    // Ensure routing hint is derived from the agent context, if any.
    let routing_hint = build_routing_hint(request.context.as_ref());
    ensure_routing_context_ready(&context, &routing_hint)
        .await
        .context("failed to prepare browser session for execution")?;

    let mut reports = Vec::new();
    let mut all_success = true;
    let mut runtime_state = FlowRuntimeState::default();

    for step in &plan.steps {
        let report = execute_step(
            dispatcher.as_ref(),
            request,
            request.task_id.clone(),
            step,
            &routing_hint,
            &options,
            &mut runtime_state,
            status_handle,
        )
        .await;
        runtime_state.absorb_step_result(step, &report);

        if matches!(report.status, StepExecutionStatus::Failed) {
            if is_weather_parse_step(step) {
                warn!(step = %report.step_id, error = ?report.error, "Weather parse failed; inserting fallback note");
                let snippet = latest_observation_snippet(&reports);
                let note_step = weather_parse_failure_note_step(
                    request,
                    report.error.as_deref(),
                    snippet.as_ref(),
                );
                let note_report = execute_note_step(request.task_id.clone(), &note_step);
                reports.push(note_report);
                break;
            }
            all_success = false;
            warn!(step = %report.step_id, error = ?report.error, "Plan step failed");
            if is_weather_guardrail_error(report.error.as_deref()) {
                reports.push(report);
                let note_step = weather_guardrail_note_step(request);
                let note_report = execute_note_step(request.task_id.clone(), &note_step);
                reports.push(note_report);
            } else {
                reports.push(report);
            }
            break;
        }

        reports.push(report);
    }

    if all_success {
        info!(task = %request.task_id.0, steps = plan.steps.len(), "Agent plan executed successfully");
    }

    let user_results = collect_user_results(&reports);
    let missing_user_result = all_success && user_results.is_empty();
    if missing_user_result {
        metrics::record_missing_user_result(request.intent.intent_kind.as_str());
    }

    Ok(FlowExecutionReport {
        success: all_success,
        steps: reports,
        user_results,
        missing_user_result,
    })
}

async fn execute_step<D: Dispatcher + ?Sized>(
    dispatcher: &D,
    request: &AgentRequest,
    task_id: TaskId,
    step: &AgentPlanStep,
    routing_hint: &Option<RoutingHint>,
    options: &FlowExecutionOptions,
    runtime_state: &mut FlowRuntimeState,
    status_handle: Option<&TaskStatusHandle>,
) -> StepExecutionReport {
    if is_note_step(step) {
        return execute_note_step(task_id, step);
    }
    let tool_kind = tool_kind_label(step);
    let total_attempts = step_attempt_budget(step, options.max_retries);
    let mut attempts = 0;
    let mut last_error: Option<String> = None;

    let mut specs = match build_dispatch_specs(step, &task_id) {
        Ok(specs) => specs,
        Err(err) => {
            let report = StepExecutionReport {
                step_id: step.id.clone(),
                title: step.title.clone(),
                tool_kind: tool_kind.clone(),
                status: StepExecutionStatus::Failed,
                attempts: 0,
                error: Some(err.to_string()),
                dispatches: Vec::new(),
                total_wait_ms: 0,
                total_run_ms: 0,
            };
            record_step_metrics(&report);
            return report;
        }
    };

    if is_observation_step(step) {
        runtime_state.apply_observation_override(&mut specs);
    }

    let mut last_dispatches: Vec<DispatchRecord> = Vec::new();

    for attempt in 0..total_attempts {
        attempts += 1;
        let mut failed = false;
        let mut step_dispatches: Vec<DispatchRecord> = Vec::new();
        let mut fallback_completed = false;

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
                    if let Some(handle) = status_handle {
                        let dispatch_index = step_dispatches.len().saturating_sub(1);
                        if let Some(record) = step_dispatches.get(dispatch_index) {
                            let evidences =
                                dispatch_artifact_values(&step.id, dispatch_index, record);
                            if !evidences.is_empty() {
                                handle.push_evidence(&evidences);
                            }
                        }
                    }
                    if !failed
                        && preview_capture_enabled()
                        && spec.label == "action"
                        && should_capture_preview_for_tool(&spec.tool)
                    {
                        if let Some(preview_dispatch) = capture_preview_frame(
                            dispatcher,
                            &task_id,
                            &step.id,
                            &output.route,
                            options,
                        )
                        .await
                        {
                            let dispatch_index = step_dispatches.len();
                            if let Some(handle) = status_handle {
                                let preview_values = dispatch_artifact_values(
                                    &step.id,
                                    dispatch_index,
                                    &preview_dispatch,
                                );
                                if !preview_values.is_empty() {
                                    handle.push_evidence(&preview_values);
                                }
                            }
                            step_dispatches.push(preview_dispatch);
                        }
                    }
                    if failed {
                        if should_attempt_url_fallback(spec, step) {
                            if let Some(dispatch) = attempt_url_navigation_fallback(
                                dispatcher,
                                &task_id,
                                routing_hint,
                                options,
                                step,
                            )
                            .await
                            {
                                step_dispatches.push(dispatch);
                                fallback_completed = true;
                                failed = false;
                                last_error = None;
                            }
                        }
                        break;
                    }
                }
                Err(err) => {
                    failed = true;
                    last_error = Some(err.to_string());
                    if should_attempt_url_fallback(spec, step) {
                        if let Some(dispatch) = attempt_url_navigation_fallback(
                            dispatcher,
                            &task_id,
                            routing_hint,
                            options,
                            step,
                        )
                        .await
                        {
                            step_dispatches.push(dispatch);
                            fallback_completed = true;
                            failed = false;
                            last_error = None;
                        }
                    }
                    break;
                }
            }
        }

        if fallback_completed {
            let (total_wait_ms, total_run_ms) = aggregate_dispatch_totals(&step_dispatches);
            let report = StepExecutionReport {
                step_id: step.id.clone(),
                title: step.title.clone(),
                tool_kind: tool_kind.clone(),
                status: StepExecutionStatus::Success,
                attempts,
                error: None,
                dispatches: step_dispatches,
                total_wait_ms,
                total_run_ms,
            };
            record_step_metrics(&report);
            return report;
        }

        if !failed {
            if let Some(guardrail) = detect_observation_guardrail_violation(
                request,
                step,
                &step_dispatches,
                runtime_state,
            ) {
                warn!(step = %step.id, reason = %guardrail, "Observation guardrail triggered");
                if guardrail.triggers_weather_recovery() {
                    if let Some(recovery) = attempt_weather_search_recovery(
                        dispatcher,
                        request,
                        &task_id,
                        routing_hint,
                        options,
                    )
                    .await
                    {
                        runtime_state.record_recovery_dispatches(&recovery);
                        step_dispatches.extend(recovery);
                    }
                }
                last_error = Some(guardrail.to_string());
                last_dispatches = step_dispatches;
                continue;
            }
            let (total_wait_ms, total_run_ms) = aggregate_dispatch_totals(&step_dispatches);
            let report = StepExecutionReport {
                step_id: step.id.clone(),
                title: step.title.clone(),
                tool_kind: tool_kind.clone(),
                status: StepExecutionStatus::Success,
                attempts,
                error: None,
                dispatches: step_dispatches,
                total_wait_ms,
                total_run_ms,
            };
            record_step_metrics(&report);
            return report;
        }

        debug!(step = %step.id, attempt, "Retrying plan step after failure");
        last_dispatches = step_dispatches;
    }

    let (total_wait_ms, total_run_ms) = aggregate_dispatch_totals(&last_dispatches);
    let report = StepExecutionReport {
        step_id: step.id.clone(),
        title: step.title.clone(),
        tool_kind: tool_kind.clone(),
        status: StepExecutionStatus::Failed,
        attempts,
        error: last_error,
        dispatches: last_dispatches,
        total_wait_ms,
        total_run_ms,
    };
    record_step_metrics(&report);
    report
}

fn plan_requires_dom_actions(plan: &AgentPlan) -> bool {
    plan.steps.iter().any(|step| {
        matches!(
            step.tool.kind,
            AgentToolKind::Click { .. }
                | AgentToolKind::TypeText { .. }
                | AgentToolKind::Select { .. }
                | AgentToolKind::Scroll { .. }
        )
    })
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

fn preview_capture_enabled() -> bool {
    *PREVIEW_CAPTURE_ENABLED
}

fn should_capture_preview_for_tool(tool: &str) -> bool {
    matches!(
        tool,
        "navigate-to-url"
            | "click"
            | "type-text"
            | "select-option"
            | "scroll-page"
            | "weather.search"
    )
}

fn preview_routing_hint(route: &ExecRoute) -> RoutingHint {
    RoutingHint {
        session: Some(route.session.clone()),
        page: Some(route.page.clone()),
        frame: Some(route.frame.clone()),
        prefer: Some(RoutePrefer::Focused),
    }
}

async fn capture_preview_frame<D: Dispatcher + ?Sized>(
    dispatcher: &D,
    task_id: &TaskId,
    step_id: &str,
    route: &ExecRoute,
    options: &FlowExecutionOptions,
) -> Option<DispatchRecord> {
    let call_options = CallOptions {
        timeout: Duration::from_secs(10),
        priority: options.priority,
        interruptible: true,
        retry: RetryOpt {
            max: 0,
            backoff: Duration::from_millis(200),
        },
    };

    let tool_call = ToolCall {
        call_id: Some(format!("{}::preview", step_id)),
        task_id: Some(task_id.clone()),
        tool: "take-screenshot".to_string(),
        payload: json!({}),
    };

    let request = DispatchRequest {
        tool_call,
        options: call_options,
        routing_hint: Some(preview_routing_hint(route)),
    };

    match dispatch_once(dispatcher, request).await {
        Ok((action_id, output)) => {
            let (wait_ms, run_ms) = timeline_metrics(&output.timeline);
            let (normalized_output, artifacts) =
                normalize_dispatch_output("preview", output.output.clone());
            let error = output.error.map(|err| err.to_string());
            Some(DispatchRecord {
                label: "preview".to_string(),
                action_id: action_id.0.clone(),
                route: output.route,
                wait_ms,
                run_ms,
                output: normalized_output,
                artifacts,
                error,
            })
        }
        Err(err) => {
            debug!(?err, "preview screenshot capture failed");
            None
        }
    }
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
    validation_condition: Option<AgentWaitCondition>,
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

#[derive(Default)]
struct FlowRuntimeState {
    weather_targets: HashMap<String, WeatherRouteRecord>,
    pending_observation_target: Option<String>,
}

#[derive(Clone, Debug)]
struct WeatherRouteRecord {
    destination_url: String,
}

impl FlowRuntimeState {
    fn route_key(route: &ExecRoute) -> String {
        format!("{}::{}", route.page.0, route.frame.0)
    }

    fn record_weather_dispatches(&mut self, dispatches: &[DispatchRecord]) {
        for dispatch in dispatches {
            let Some(payload) = dispatch_payload(dispatch) else {
                continue;
            };
            let Some(status) = payload.get("status").and_then(Value::as_str) else {
                continue;
            };
            if status != "weather_ready" {
                continue;
            }
            if let Some(destination) = payload.get("destination_url").and_then(Value::as_str) {
                self.weather_targets.insert(
                    Self::route_key(&dispatch.route),
                    WeatherRouteRecord {
                        destination_url: destination.to_string(),
                    },
                );
                self.pending_observation_target = Some(destination.to_string());
            }
        }
    }

    fn absorb_step_result(&mut self, step: &AgentPlanStep, report: &StepExecutionReport) {
        if matches!(
            step.tool.kind,
            AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case("weather.search")
        ) {
            self.record_weather_dispatches(&report.dispatches);
        }
    }

    fn expected_url_override(&self, dispatches: &[DispatchRecord]) -> Option<String> {
        let route = dispatches
            .iter()
            .find(|dispatch| dispatch.label == "action")
            .map(|dispatch| dispatch.route.clone())?;
        self.weather_targets
            .get(&Self::route_key(&route))
            .map(|record| record.destination_url.clone())
    }

    fn record_recovery_dispatches(&mut self, dispatches: &[DispatchRecord]) {
        self.record_weather_dispatches(dispatches);
    }

    fn apply_observation_override(&mut self, specs: &mut [DispatchSpec]) {
        let Some(url) = self.pending_observation_target.take() else {
            return;
        };
        let Some(first) = specs.first_mut() else {
            return;
        };
        if !first.tool.eq_ignore_ascii_case(OBSERVATION_CANONICAL) {
            return;
        }
        if let Some(object) = first.payload.as_object_mut() {
            object.insert("url".to_string(), Value::String(url));
        }
    }
}

fn is_note_step(step: &AgentPlanStep) -> bool {
    matches!(
        step.tool.kind,
        AgentToolKind::Custom { ref name, .. } if is_note_tool_name(name)
    )
}

fn is_note_tool_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("agent.note") || name.to_ascii_lowercase().ends_with("note")
}

fn execute_note_step(task_id: TaskId, step: &AgentPlanStep) -> StepExecutionReport {
    let note_text = extract_note_message(step);
    let dispatch = DispatchRecord {
        label: "agent.note".to_string(),
        action_id: format!("note-{}", ActionId::new().0),
        route: synthetic_route(),
        wait_ms: 0,
        run_ms: 0,
        output: Some(json!({
            "output": {
                "status": "noted",
                "note": note_text,
                "title": step.title,
                "detail": step.detail,
                "task_id": task_id.0,
            }
        })),
        artifacts: Vec::new(),
        error: None,
    };
    let report = StepExecutionReport {
        step_id: step.id.clone(),
        title: step.title.clone(),
        tool_kind: tool_kind_label(step),
        status: StepExecutionStatus::Success,
        attempts: 1,
        error: None,
        dispatches: vec![dispatch],
        total_wait_ms: 0,
        total_run_ms: 0,
    };
    record_step_metrics(&report);
    report
}

fn extract_note_message(step: &AgentPlanStep) -> String {
    if let AgentToolKind::Custom { payload, .. } = &step.tool.kind {
        if let Some(message) = payload.get("message").and_then(Value::as_str) {
            let trimmed = message.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        if let Some(message) = payload.get("note").and_then(Value::as_str) {
            let trimmed = message.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    if !step.detail.trim().is_empty() {
        step.detail.trim().to_string()
    } else {
        step.title.clone()
    }
}

fn synthetic_route() -> ExecRoute {
    ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new())
}

fn normalize_dispatch_output(
    label: &str,
    output: Option<Value>,
) -> (Option<Value>, Vec<RunArtifact>) {
    let mut artifacts = Vec::new();
    let Some(mut value) = output else {
        return (None, artifacts);
    };

    collect_artifacts(label, &mut value, &mut artifacts);
    (Some(value), artifacts)
}

fn collect_artifacts(label: &str, value: &mut Value, artifacts: &mut Vec<RunArtifact>) {
    let Value::Object(obj) = value else {
        return;
    };

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

    // Scheduler dispatches wrap the actual tool payload under `output`, so recurse into it.
    if let Some(inner) = obj.get_mut("output") {
        collect_artifacts(label, inner, artifacts);
    }
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

fn aggregate_dispatch_totals(dispatches: &[DispatchRecord]) -> (u64, u64) {
    dispatches
        .iter()
        .fold((0u64, 0u64), |(wait_acc, run_acc), dispatch| {
            (wait_acc + dispatch.wait_ms, run_acc + dispatch.run_ms)
        })
}

fn dispatch_artifact_values(
    step_id: &str,
    dispatch_index: usize,
    dispatch: &DispatchRecord,
) -> Vec<Value> {
    dispatch
        .artifacts
        .iter()
        .map(|artifact| {
            json!({
                "step_id": step_id,
                "dispatch_label": dispatch.label,
                "dispatch_index": dispatch_index,
                "action_id": dispatch.action_id,
                "route": {
                    "session": dispatch.route.session.0,
                    "page": dispatch.route.page.0,
                    "frame": dispatch.route.frame.0,
                },
                "label": artifact.label,
                "content_type": artifact.content_type,
                "byte_len": artifact.byte_len,
                "data_base64": artifact.data_base64,
                "filename": artifact.filename,
            })
        })
        .collect()
}

fn collect_user_results(steps: &[StepExecutionReport]) -> Vec<UserResult> {
    let mut results = Vec::new();
    for step in steps {
        let tool = step.tool_kind.to_ascii_lowercase();
        if is_note_tool_name(&tool) {
            if step.dispatches.is_empty() {
                results.push(UserResult {
                    step_id: step.step_id.clone(),
                    step_title: step.title.clone(),
                    kind: UserResultKind::Note,
                    schema: None,
                    content: Value::String(step.title.clone()),
                    artifact_path: None,
                });
            } else {
                for dispatch in &step.dispatches {
                    if let Some(payload) = dispatch_payload(dispatch) {
                        if let Some(note) = payload.get("note").and_then(Value::as_str) {
                            results.push(UserResult {
                                step_id: step.step_id.clone(),
                                step_title: step.title.clone(),
                                kind: UserResultKind::Note,
                                schema: None,
                                content: Value::String(note.to_string()),
                                artifact_path: None,
                            });
                        }
                    }
                }
            }
            continue;
        }

        if tool.starts_with("data.parse.") {
            for dispatch in &step.dispatches {
                if let Some(payload) = dispatch_payload(dispatch) {
                    if let Some(result_value) = payload.get("result") {
                        results.push(UserResult {
                            step_id: step.step_id.clone(),
                            step_title: step.title.clone(),
                            kind: UserResultKind::Structured,
                            schema: payload
                                .get("schema")
                                .and_then(Value::as_str)
                                .map(|s| s.to_string()),
                            content: result_value.clone(),
                            artifact_path: None,
                        });
                    }
                }
            }
            continue;
        }

        if tool.starts_with("data.deliver.") {
            for dispatch in &step.dispatches {
                if let Some(payload) = dispatch_payload(dispatch) {
                    if let Some(path) = payload.get("artifact_path").and_then(Value::as_str) {
                        results.push(UserResult {
                            step_id: step.step_id.clone(),
                            step_title: step.title.clone(),
                            kind: UserResultKind::Artifact,
                            schema: payload
                                .get("schema")
                                .and_then(Value::as_str)
                                .map(|s| s.to_string()),
                            content: Value::Null,
                            artifact_path: Some(path.to_string()),
                        });
                    }
                }
            }
        }
    }
    results
}

fn dispatch_payload(dispatch: &DispatchRecord) -> Option<&Value> {
    dispatch.output.as_ref()?.get("output")
}

fn should_attempt_url_fallback(spec: &DispatchSpec, step: &AgentPlanStep) -> bool {
    matches!(
        spec.validation_condition,
        Some(AgentWaitCondition::UrlMatches(_)) | Some(AgentWaitCondition::UrlEquals(_))
    ) && step_expected_url(step).is_some()
}

async fn attempt_url_navigation_fallback<D: Dispatcher + ?Sized>(
    dispatcher: &D,
    task_id: &TaskId,
    routing_hint: &Option<RoutingHint>,
    options: &FlowExecutionOptions,
    step: &AgentPlanStep,
) -> Option<DispatchRecord> {
    let target_url = step_expected_url(step)?;
    if target_url.trim().is_empty() {
        return None;
    }

    dispatch_ad_hoc_tool(
        dispatcher,
        task_id,
        routing_hint,
        options,
        "fallback-navigate",
        "navigate-to-url",
        json!({
            "url": target_url,
            "wait_tier": "domready",
        }),
        None,
    )
    .await
}

async fn attempt_weather_search_recovery<D: Dispatcher + ?Sized>(
    dispatcher: &D,
    request: &AgentRequest,
    task_id: &TaskId,
    routing_hint: &Option<RoutingHint>,
    options: &FlowExecutionOptions,
) -> Option<Vec<DispatchRecord>> {
    if !requires_weather_pipeline(request) {
        return None;
    }

    let mut records = Vec::new();
    let macro_step = dispatch_ad_hoc_tool(
        dispatcher,
        task_id,
        routing_hint,
        options,
        "weather-search-macro",
        "weather.search",
        json!({ "query": weather_query_text(request) }),
        Some(25_000),
    )
    .await?;
    records.push(macro_step);

    Some(records)
}

async fn dispatch_ad_hoc_tool<D: Dispatcher + ?Sized>(
    dispatcher: &D,
    task_id: &TaskId,
    routing_hint: &Option<RoutingHint>,
    options: &FlowExecutionOptions,
    label: &str,
    tool_id: &str,
    payload: Value,
    timeout_override: Option<u64>,
) -> Option<DispatchRecord> {
    let call_options = CallOptions {
        timeout: Duration::from_millis(timeout_override.unwrap_or(DEFAULT_TIMEOUT_MS)),
        priority: options.priority,
        interruptible: true,
        retry: RetryOpt {
            max: 0,
            backoff: Duration::from_millis(200),
        },
    };

    let tool_call = ToolCall {
        call_id: Some(format!("{}::{label}", task_id.0)),
        task_id: Some(task_id.clone()),
        tool: tool_id.to_string(),
        payload,
    };

    let request = DispatchRequest {
        tool_call,
        options: call_options,
        routing_hint: routing_hint.clone(),
    };

    match dispatch_once(dispatcher, request).await {
        Ok((action_id, output)) => {
            let (wait_ms, run_ms) = timeline_metrics(&output.timeline);
            let (normalized_output, artifacts) =
                normalize_dispatch_output(label, output.output.clone());
            Some(DispatchRecord {
                label: label.to_string(),
                action_id: action_id.0,
                route: output.route,
                wait_ms,
                run_ms,
                output: normalized_output,
                artifacts,
                error: output.error.map(|err| err.to_string()),
            })
        }
        Err(err) => {
            warn!(stage = label, error = %err, "Ad-hoc dispatch failed");
            None
        }
    }
}

fn step_attempt_budget(step: &AgentPlanStep, configured_max: u8) -> u8 {
    let base = configured_max.max(1);
    if matches!(
        step.tool.kind,
        AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case(OBSERVATION_CANONICAL)
    ) {
        base.max(2)
    } else {
        base
    }
}

fn detect_observation_guardrail_violation(
    request: &AgentRequest,
    step: &AgentPlanStep,
    dispatches: &[DispatchRecord],
    runtime_state: &FlowRuntimeState,
) -> Option<ObservationGuardrail> {
    if !is_observation_step(step) {
        return None;
    }

    let observation = observation_snapshot_from_dispatches(dispatches)?;
    let actual_url = observation_source_url(observation)?;
    let expected_override = runtime_state.expected_url_override(dispatches);

    if let Some(expected) = expected_override.or_else(|| step_expected_url(step)) {
        if !urls_equivalent(&actual_url, &expected) {
            return Some(ObservationGuardrail::UrlMismatch {
                expected,
                actual: actual_url,
            });
        }
    }

    if requires_weather_pipeline(request)
        && (is_baidu_homepage(&actual_url)
            || (observation_looks_like_baidu_home(observation)
                && is_probably_baidu_home_without_results(&actual_url)))
    {
        return Some(ObservationGuardrail::WeatherBaiduHome { actual: actual_url });
    }

    if let Some(reason) = observation_block_reason(observation, &actual_url) {
        return Some(ObservationGuardrail::Blocked { reason });
    }

    None
}

fn is_observation_step(step: &AgentPlanStep) -> bool {
    matches!(
        step.tool.kind,
        AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case(OBSERVATION_CANONICAL)
    )
}

#[derive(Debug, Clone)]
enum ObservationGuardrail {
    UrlMismatch { expected: String, actual: String },
    WeatherBaiduHome { actual: String },
    Blocked { reason: String },
}

impl ObservationGuardrail {
    fn triggers_weather_recovery(&self) -> bool {
        matches!(
            self,
            ObservationGuardrail::WeatherBaiduHome { .. } | ObservationGuardrail::Blocked { .. }
        )
    }
}

impl fmt::Display for ObservationGuardrail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObservationGuardrail::UrlMismatch { expected, actual } => write!(
                f,
                "observation captured {actual} but expected page resolved to {expected}"
            ),
            ObservationGuardrail::WeatherBaiduHome { actual } => {
                write!(f, "{} ({actual})", WEATHER_GUARDRAIL_MESSAGE)
            }
            ObservationGuardrail::Blocked { reason } => {
                write!(f, "Observation blocked: {reason}")
            }
        }
    }
}

fn observation_looks_like_baidu_home(observation: &Value) -> bool {
    let text = observation_text_snippet(observation).to_ascii_lowercase();
    let title = observation_primary_title(observation).to_ascii_lowercase();

    text.contains("百度首页")
        || text.contains("baidu")
        || title.contains("百度一下")
        || title.contains("baidu")
}

fn is_probably_baidu_home_without_results(url: &str) -> bool {
    if let Ok(parsed) = Url::parse(url) {
        if let Some(domain) = parsed.domain() {
            if domain.eq_ignore_ascii_case("www.baidu.com") {
                let path = parsed.path();
                return path.trim_matches('/').is_empty() || !path.contains("/s");
            }
        }
    }
    false
}

fn observation_snapshot_from_dispatches(dispatches: &[DispatchRecord]) -> Option<&Value> {
    dispatches
        .iter()
        .find(|dispatch| dispatch.label == "action")
        .and_then(dispatch_payload)
        .and_then(|payload| payload.get("observation"))
}

fn observation_source_url(observation: &Value) -> Option<String> {
    if let Some(url) = observation.get("url").and_then(Value::as_str) {
        return Some(url.to_string());
    }
    observation
        .get("data")
        .and_then(|data| data.get("url"))
        .and_then(Value::as_str)
        .map(|value| value.to_string())
}

fn step_expected_url(step: &AgentPlanStep) -> Option<String> {
    step.metadata
        .get(EXPECTED_URL_METADATA_KEY)
        .and_then(Value::as_str)
        .map(|value| value.to_string())
}

fn urls_equivalent(actual: &str, expected: &str) -> bool {
    if expected.is_empty() {
        return true;
    }
    if actual.contains(expected) {
        return true;
    }
    match (Url::parse(actual), Url::parse(expected)) {
        (Ok(actual_url), Ok(expected_url)) => {
            let expected_path = expected_url.path().trim_end_matches('/');
            let actual_path = actual_url.path();
            actual_url.domain() == expected_url.domain()
                && (expected_path.is_empty()
                    || actual_path.starts_with(expected_path)
                    || actual_url.as_str().starts_with(expected_url.as_str()))
        }
        _ => false,
    }
}

fn observation_primary_title(observation: &Value) -> String {
    observation
        .get("title")
        .or_else(|| observation.get("data").and_then(|data| data.get("title")))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn observation_text_snippet(observation: &Value) -> String {
    observation
        .get("text_sample")
        .or_else(|| observation.get("summary"))
        .or_else(|| {
            observation
                .get("data")
                .and_then(|data| data.get("text_sample").or_else(|| data.get("summary")))
        })
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn observation_block_reason(observation: &Value, actual_url: &str) -> Option<String> {
    let title = observation_primary_title(observation);
    let text = observation_text_snippet(observation);
    if title.is_empty() && text.is_empty() {
        return None;
    }
    detect_block_reason(&title, &text, Some(actual_url))
}

fn is_baidu_homepage(url: &str) -> bool {
    if let Ok(parsed) = Url::parse(url) {
        if let Some(domain) = parsed.domain() {
            if domain.eq_ignore_ascii_case("www.baidu.com") {
                let path = parsed.path().trim_matches('/');
                return path.is_empty();
            }
        }
    }
    false
}

fn weather_guardrail_note_step(request: &AgentRequest) -> AgentPlanStep {
    let guidance = format!(
        "未能获取结果，请检查搜索页面：{}",
        weather_search_url(request)
    );
    AgentPlanStep::new(
        format!("guardrail-note-{}", request.task_id.0),
        "天气结果未获取".to_string(),
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "agent.note".to_string(),
                payload: json!({
                    "title": "天气查询受阻",
                    "detail": guidance,
                    "message": guidance,
                }),
            },
            wait: WaitMode::None,
            timeout_ms: Some(2_000),
        },
    )
    .with_detail(guidance)
}

fn is_weather_guardrail_error(message: Option<&str>) -> bool {
    if let Some(text) = message {
        text.contains(WEATHER_GUARDRAIL_MESSAGE)
    } else {
        false
    }
}

fn is_weather_parse_step(step: &AgentPlanStep) -> bool {
    matches!(
        step.tool.kind,
        AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case("data.parse.weather")
    )
}

fn weather_parse_failure_note_step(
    request: &AgentRequest,
    error: Option<&str>,
    snippet: Option<&WeatherObservationSnippet>,
) -> AgentPlanStep {
    let mut sections = Vec::new();
    let mut headline = "未能解析天气结构化数据".to_string();
    if let Some(err) = error {
        if !err.trim().is_empty() {
            headline.push_str(&format!("：{}", err));
        }
    }
    sections.push(headline);

    if let Some(snippet) = snippet {
        let mut snippet_text = String::new();
        if !snippet.title.is_empty() {
            snippet_text.push_str(&format!("搜索摘要《{}》\n", snippet.title));
        }
        if !snippet.sample.is_empty() {
            snippet_text.push_str(&truncate_text(&snippet.sample, 220));
        }
        if let Some(url) = &snippet.url {
            snippet_text.push_str(&format!("\n参考链接：{}", url));
        }
        sections.push(snippet_text);
    }

    let search_url = weather_search_url(request);
    sections.push(format!("可直接打开天气搜索结果：{}", search_url));

    let detail = sections.join("\n\n");
    AgentPlanStep::new(
        format!("guardrail-note-{}", request.task_id.0),
        "天气信息获取失败".to_string(),
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "agent.note".to_string(),
                payload: json!({
                    "title": "天气查询受阻",
                    "detail": detail,
                    "message": detail,
                }),
            },
            wait: WaitMode::None,
            timeout_ms: Some(2_000),
        },
    )
    .with_detail(detail)
}

fn latest_observation_snippet(
    reports: &[StepExecutionReport],
) -> Option<WeatherObservationSnippet> {
    for report in reports.iter().rev() {
        if !report.tool_kind.eq_ignore_ascii_case("data.extract-site") {
            continue;
        }
        if let Some(snippet) = observation_snippet_from_dispatches(&report.dispatches) {
            return Some(snippet);
        }
    }
    None
}

fn observation_snippet_from_dispatches(
    dispatches: &[DispatchRecord],
) -> Option<WeatherObservationSnippet> {
    let observation = observation_snapshot_from_dispatches(dispatches)?;
    Some(WeatherObservationSnippet {
        title: observation
            .get("title")
            .or_else(|| observation.get("data").and_then(|data| data.get("title")))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        url: observation
            .get("url")
            .and_then(Value::as_str)
            .map(|value| value.to_string())
            .or_else(|| {
                observation
                    .get("data")
                    .and_then(|data| data.get("url"))
                    .and_then(Value::as_str)
                    .map(|value| value.to_string())
            }),
        sample: observation
            .get("text_sample")
            .or_else(|| {
                observation
                    .get("data")
                    .and_then(|data| data.get("text_sample"))
            })
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

fn truncate_text(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut truncated = String::with_capacity(max_chars + 3);
    for (idx, ch) in input.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        truncated.push(ch);
    }
    truncated.push('…');
    truncated
}

struct WeatherObservationSnippet {
    title: String,
    url: Option<String>,
    sample: String,
}

#[cfg(test)]
mod tests {
    use super::EXPECTED_URL_METADATA_KEY;
    use super::*;
    use agent_core::{
        plan::{AgentPlanStep, AgentTool, AgentToolKind},
        ConversationRole, ConversationTurn,
    };
    use serde_json::json;

    fn sample_request(goal: &str) -> AgentRequest {
        let mut request = AgentRequest::new(TaskId::new(), goal.to_string());
        request.push_turn(ConversationTurn::new(ConversationRole::User, goal));
        request
    }

    fn observation_step(expected_url: Option<&str>) -> AgentPlanStep {
        let mut step = AgentPlanStep::new(
            "observe-1",
            "采集网页",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "data.extract-site".to_string(),
                    payload: json!({}),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(10_000),
            },
        );
        if let Some(url) = expected_url {
            step.metadata.insert(
                EXPECTED_URL_METADATA_KEY.to_string(),
                json!(url.to_string()),
            );
        }
        step
    }

    fn observation_dispatches(url: &str) -> Vec<DispatchRecord> {
        observation_dispatches_with_route(url, synthetic_route())
    }

    fn observation_dispatches_with_route(url: &str, route: ExecRoute) -> Vec<DispatchRecord> {
        vec![DispatchRecord {
            label: "action".to_string(),
            action_id: ActionId::new().0,
            route,
            wait_ms: 0,
            run_ms: 0,
            output: Some(json!({
                "output": {
                    "observation": {
                        "url": url
                    }
                }
            })),
            artifacts: Vec::new(),
            error: None,
        }]
    }

    #[test]
    fn guard_respects_weather_override() {
        let request = sample_request("查询天气");
        let mut state = FlowRuntimeState::default();
        let route = synthetic_route();
        state.record_weather_dispatches(&vec![DispatchRecord {
            label: "action".to_string(),
            action_id: ActionId::new().0,
            route: route.clone(),
            wait_ms: 0,
            run_ms: 0,
            output: Some(json!({
                "output": {
                    "status": "weather_ready",
                    "destination_url": "https://www.moji.com",
                    "page_snapshot": {
                        "url": "https://www.moji.com"
                    }
                }
            })),
            artifacts: Vec::new(),
            error: None,
        }]);

        let step = observation_step(Some("https://weather.com"));
        let dispatches = observation_dispatches_with_route("https://www.moji.com", route);
        let violation =
            detect_observation_guardrail_violation(&request, &step, &dispatches, &state);
        assert!(violation.is_none());
    }

    #[test]
    fn guard_detects_blocked_snapshot() {
        let request = sample_request("查询天气");
        let step = observation_step(None);
        let mut dispatches = observation_dispatches("https://wappass.baidu.com");
        if let Some(entry) = dispatches.first_mut() {
            entry.output = Some(json!({
                "output": {
                    "observation": {
                        "url": "https://wappass.baidu.com",
                        "title": "403 Forbidden",
                        "text_sample": "verify you're a human"
                    }
                }
            }));
        }
        let violation = detect_observation_guardrail_violation(
            &request,
            &step,
            &dispatches,
            &FlowRuntimeState::default(),
        )
        .expect("blocked");
        assert!(matches!(violation, ObservationGuardrail::Blocked { .. }));
        assert!(violation.triggers_weather_recovery());
    }

    #[test]
    fn guard_flags_baidu_home_for_weather_requests() {
        let request = sample_request("查询今天天气");
        let step = observation_step(None);
        let dispatches = observation_dispatches("https://www.baidu.com/");
        let violation = detect_observation_guardrail_violation(
            &request,
            &step,
            &dispatches,
            &FlowRuntimeState::default(),
        )
        .expect("weather guardrail");
        assert!(matches!(
            violation,
            ObservationGuardrail::WeatherBaiduHome { .. }
        ));
        assert!(violation.to_string().contains(WEATHER_GUARDRAIL_MESSAGE));
    }

    #[test]
    fn guard_checks_expected_url_mismatch() {
        let request = sample_request("查看页面");
        let step = observation_step(Some("https://www.baidu.com/s?wd=test"));
        let dispatches = observation_dispatches("https://www.baidu.com/");
        let violation = detect_observation_guardrail_violation(
            &request,
            &step,
            &dispatches,
            &FlowRuntimeState::default(),
        )
        .expect("url guardrail");
        assert!(violation
            .to_string()
            .contains("observation captured https://www.baidu.com/"));
    }

    #[test]
    fn guard_allows_matching_expected_url() {
        let request = sample_request("查看页面");
        let step = observation_step(Some("https://www.baidu.com"));
        let dispatches = observation_dispatches("https://www.baidu.com/s?wd=test");
        assert!(detect_observation_guardrail_violation(
            &request,
            &step,
            &dispatches,
            &FlowRuntimeState::default()
        )
        .is_none());
    }

    #[test]
    fn url_validation_dispatch_maps_to_wait_for_condition() {
        let condition = AgentWaitCondition::UrlMatches("https://example.com/results".to_string());
        let (tool, payload, timeout) = wait_condition_dispatch(&condition).expect("dispatch");
        assert_eq!(tool, "wait-for-condition");
        assert_eq!(
            payload,
            json!({ "expect": { "url_pattern": "https://example.com/results" } })
        );
        assert!(timeout.is_none());
    }

    #[test]
    fn url_equals_dispatch_maps_to_wait_for_condition() {
        let condition = AgentWaitCondition::UrlEquals("https://example.com/results".to_string());
        let (tool, payload, timeout) = wait_condition_dispatch(&condition).expect("dispatch");
        assert_eq!(tool, "wait-for-condition");
        assert_eq!(
            payload,
            json!({ "expect": { "url_equals": "https://example.com/results" } })
        );
        assert!(timeout.is_none());
    }
}

fn record_step_metrics(report: &StepExecutionReport) {
    let result = match report.status {
        StepExecutionStatus::Success => "success",
        StepExecutionStatus::Failed => "failed",
    };
    metrics::observe_execution_step(
        &report.tool_kind,
        result,
        report.total_wait_ms,
        report.total_run_ms,
        report.attempts as u64,
    );
}

fn build_dispatch_specs(step: &AgentPlanStep, task_id: &TaskId) -> Result<Vec<DispatchSpec>> {
    let mut specs = Vec::new();
    specs.push(step_to_spec(step, task_id)?);

    for (idx, validation) in step.validations.iter().enumerate() {
        if let Some(spec) = validation_to_spec(step, validation, idx)? {
            specs.push(spec);
        }
    }

    Ok(specs)
}

fn step_to_spec(step: &AgentPlanStep, task_id: &TaskId) -> Result<DispatchSpec> {
    let (tool, mut payload) = tool_payload(&step.tool, task_id)?;
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
        validation_condition: None,
    })
}

fn validation_to_spec(
    step: &AgentPlanStep,
    validation: &AgentValidation,
    index: usize,
) -> Result<Option<DispatchSpec>> {
    match wait_condition_dispatch(&validation.condition) {
        Ok((tool, payload, timeout_ms)) => Ok(Some(DispatchSpec {
            tool,
            payload,
            timeout_ms,
            label: format!("validation-{}", index),
            validation_condition: Some(validation.condition.clone()),
        })),
        Err(err) => {
            warn!(step = %step.id, ?err, "Skipping unsupported validation condition");
            Ok(None)
        }
    }
}

fn tool_payload(tool: &AgentTool, task_id: &TaskId) -> Result<(String, Value)> {
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
        AgentToolKind::Custom { name, payload } => {
            let normalized = name.trim();
            if normalized.starts_with("data.parse.")
                || normalized == "data.extract-site"
                || normalized == "data.deliver.structured"
            {
                Ok((
                    normalized.to_string(),
                    merge_custom_payload(payload, task_id),
                ))
            } else if normalized.eq_ignore_ascii_case("weather.search") {
                Ok((
                    "weather.search".to_string(),
                    merge_custom_payload(payload, task_id),
                ))
            } else if normalized == "deliver" {
                Ok((
                    "data.deliver.structured".to_string(),
                    merge_custom_payload(payload, task_id),
                ))
            } else {
                Err(anyhow!(
                    "Custom tool '{}' is not supported for automated execution",
                    name
                ))
            }
        }
    }
}

fn tool_kind_label(step: &AgentPlanStep) -> String {
    match &step.tool.kind {
        AgentToolKind::Navigate { .. } => "navigate",
        AgentToolKind::Click { .. } => "click",
        AgentToolKind::TypeText { .. } => "type_text",
        AgentToolKind::Select { .. } => "select",
        AgentToolKind::Scroll { .. } => "scroll",
        AgentToolKind::Wait { .. } => "wait",
        AgentToolKind::Custom { name, .. } => name.as_str(),
    }
    .to_string()
}

fn wait_tool_payload(condition: &AgentWaitCondition) -> Result<(String, Value)> {
    wait_condition_dispatch(condition).map(|(tool, payload, _)| (tool, payload))
}

fn wait_condition_dispatch(condition: &AgentWaitCondition) -> Result<(String, Value, Option<u64>)> {
    match condition {
        AgentWaitCondition::ElementVisible(locator) => Ok((
            "wait-for-element".to_string(),
            json!({
                "target": { "anchor": locator_to_json(locator) },
                "condition": { "kind": "visible" }
            }),
            None,
        )),
        AgentWaitCondition::ElementHidden(locator) => Ok((
            "wait-for-element".to_string(),
            json!({
                "target": { "anchor": locator_to_json(locator) },
                "condition": { "kind": "hidden" }
            }),
            None,
        )),
        AgentWaitCondition::NetworkIdle(ms) => Ok((
            "wait-for-condition".to_string(),
            json!({ "expect": { "net": { "quiet_ms": ms } } }),
            Some(*ms),
        )),
        AgentWaitCondition::Duration(ms) => Ok((
            "wait-for-condition".to_string(),
            json!({ "expect": { "duration_ms": ms } }),
            Some((*ms).saturating_add(1_000)),
        )),
        AgentWaitCondition::UrlMatches(pattern) => Ok((
            "wait-for-condition".to_string(),
            json!({ "expect": { "url_pattern": pattern } }),
            None,
        )),
        AgentWaitCondition::UrlEquals(expected) => Ok((
            "wait-for-condition".to_string(),
            json!({ "expect": { "url_equals": expected } }),
            None,
        )),
        AgentWaitCondition::TitleMatches(pattern) => Ok((
            "wait-for-condition".to_string(),
            json!({ "expect": { "title_pattern": pattern } }),
            None,
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

fn merge_custom_payload(payload: &Value, task_id: &TaskId) -> Value {
    let mut obj = payload.as_object().cloned().unwrap_or_default();
    obj.entry("task_id".to_string())
        .or_insert(json!(task_id.0.clone()));
    Value::Object(obj)
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

async fn ensure_routing_context_ready(
    context: &Arc<AppContext>,
    hint: &Option<RoutingHint>,
) -> Result<()> {
    let registry = context.registry();
    if let Some(session_id) = hint.as_ref().and_then(|h| h.session.clone()) {
        ensure_session_ready(&registry, &session_id).await
    } else {
        ensure_any_session_ready(&registry).await
    }
}

async fn ensure_session_ready(registry: &Arc<RegistryImpl>, session_id: &SessionId) -> Result<()> {
    let sessions = registry.session_list().await;
    let Some(ctx) = sessions.into_iter().find(|ctx| ctx.id == *session_id) else {
        return Err(anyhow!(
            "session {} not registered in browser registry",
            session_id.0
        ));
    };

    if ctx.focused_page.is_some() {
        return Ok(());
    }

    let page = registry
        .page_open(session_id.clone())
        .await
        .map_err(|err| anyhow!("failed to open page for session {}: {}", session_id.0, err))?;
    registry.page_focus(page.clone()).await.map_err(|err| {
        anyhow!(
            "failed to focus page {} for session {}: {}",
            page.0,
            session_id.0,
            err
        )
    })?;

    Ok(())
}

async fn ensure_any_session_ready(registry: &Arc<RegistryImpl>) -> Result<()> {
    let sessions = registry.session_list().await;
    if sessions.is_empty() {
        let session = registry
            .session_create("agent-runtime")
            .await
            .map_err(|err| anyhow!("failed to create fallback session: {}", err))?;
        return ensure_session_ready(registry, &session).await;
    }

    if sessions.iter().any(|ctx| ctx.focused_page.is_some()) {
        return Ok(());
    }

    let fallback = sessions[0].id.clone();
    ensure_session_ready(registry, &fallback).await
}
