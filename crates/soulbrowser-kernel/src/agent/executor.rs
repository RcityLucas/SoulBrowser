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
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::Serialize;
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
    judge::{self, JudgeVerdict},
    manual_override::{
        ManualOverridePhase, ManualOverrideSnapshot, ManualRouteContext, ManualSessionManager,
    },
    metrics,
    task_status::{AgentHistoryEntry, AgentHistoryStatus, TaskLogLevel, TaskStatusHandle},
    telemetry,
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
    pub observation_summary: Option<String>,
    pub blocker_kind: Option<String>,
    pub agent_state: Option<Value>,
}

/// Aggregate execution report for an entire plan.
#[derive(Clone, Debug)]
pub struct FlowExecutionReport {
    pub success: bool,
    pub steps: Vec<StepExecutionReport>,
    pub user_results: Vec<UserResult>,
    pub missing_user_result: bool,
    pub memory_log: Vec<ExecutionMemoryEntry>,
    pub judge_verdict: Option<JudgeVerdict>,
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

    let manual_controller = ManualOverrideController::new(
        context.manual_session_manager(),
        status_handle.cloned(),
        request.task_id.clone(),
    );

    let mut reports = Vec::new();
    let mut all_success = true;
    let mut runtime_state = FlowRuntimeState::default();

    for step in &plan.steps {
        manual_controller
            .wait_if_active(dispatcher.as_ref(), &options)
            .await?;

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
        telemetry::emit_step_report(context.tenant_id(), &request.task_id, &report);
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

    manual_controller
        .wait_if_active(dispatcher.as_ref(), &options)
        .await?;

    let user_results = collect_user_results(&reports);
    let missing_user_result = all_success && user_results.is_empty();
    if missing_user_result {
        metrics::record_missing_user_result(request.intent.intent_kind.as_str());
    }

    let memory_log = runtime_state.memory_log();
    let mut report = FlowExecutionReport {
        success: all_success,
        steps: reports,
        user_results,
        missing_user_result,
        memory_log,
        judge_verdict: None,
    };
    let verdict = judge::evaluate_plan(request, &report);
    report.judge_verdict = Some(verdict);
    Ok(report)
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
                observation_summary: None,
                blocker_kind: None,
                agent_state: agent_state_metadata(step),
            };
            record_step_metrics(&report);
            return report;
        }
    };

    if is_observation_step(step) {
        runtime_state.apply_observation_override(&mut specs);
    }

    let mut last_dispatches: Vec<DispatchRecord> = Vec::new();
    let mut guardrail_context: Option<GuardrailContext> = None;

    'attempts: for attempt in 0..total_attempts {
        attempts += 1;
        guardrail_context = None;
        runtime_state.apply_auto_act_exclusions(step, &mut specs);
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
                routing_hint: runtime_state.resolve_routing_hint(routing_hint),
            };

            match dispatch_once(dispatcher, request).await {
                Ok((action_id, output)) => {
                    let (normalized_output, artifacts) =
                        normalize_dispatch_output(&spec.label, output.output.clone());
                    runtime_state.record_route_hint(&output.route);
                    let error = output.error.as_ref().map(|err| err.to_string());
                    if let Some(err) = error.clone() {
                        failed = true;
                        last_error = Some(err);
                    }
                    let record = dispatch_record_from_output(
                        spec.label.clone(),
                        &action_id,
                        &output,
                        normalized_output,
                        artifacts,
                        error,
                    );
                    runtime_state.record_auto_act_exclusions(step, &record);
                    step_dispatches.push(record);
                    let dispatch_index = step_dispatches.len().saturating_sub(1);
                    if let Some(handle) = status_handle {
                        if let Some(record) = step_dispatches.get(dispatch_index) {
                            let evidences =
                                dispatch_artifact_values(&step.id, dispatch_index, record);
                            if !evidences.is_empty() {
                                handle.push_evidence(&evidences);
                            }
                        }
                    }
                    if !failed {
                        if let Some(record) = step_dispatches.get(dispatch_index) {
                            if let Some(reason) = tool_failure_from_output(record) {
                                if let Some(handle) = status_handle {
                                    emit_auto_act_state(handle, record);
                                }
                                failed = true;
                                last_error = Some(reason);
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
                            let fallback_hint = runtime_state.resolve_routing_hint(routing_hint);
                            if let Some(dispatch) = attempt_url_navigation_fallback(
                                dispatcher,
                                &task_id,
                                &fallback_hint,
                                options,
                                step,
                            )
                            .await
                            {
                                runtime_state.record_route_hint(&dispatch.route);
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
                        let fallback_hint = runtime_state.resolve_routing_hint(routing_hint);
                        if let Some(dispatch) = attempt_url_navigation_fallback(
                            dispatcher,
                            &task_id,
                            &fallback_hint,
                            options,
                            step,
                        )
                        .await
                        {
                            runtime_state.record_route_hint(&dispatch.route);
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
            let summary = success_summary(step, &step_dispatches);
            let (total_wait_ms, total_run_ms) = aggregate_dispatch_totals(&step_dispatches);
            let dispatches = step_dispatches;
            runtime_state.clear_auto_act_state(&step.id);
            let report = StepExecutionReport {
                step_id: step.id.clone(),
                title: step.title.clone(),
                tool_kind: tool_kind.clone(),
                status: StepExecutionStatus::Success,
                attempts,
                error: None,
                dispatches,
                total_wait_ms,
                total_run_ms,
                observation_summary: summary,
                blocker_kind: None,
                agent_state: agent_state_metadata(step),
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
                    let weather_hint = runtime_state.resolve_routing_hint(routing_hint);
                    if let Some(recovery) = attempt_weather_search_recovery(
                        dispatcher,
                        request,
                        &task_id,
                        &weather_hint,
                        options,
                    )
                    .await
                    {
                        runtime_state.record_recovery_dispatches(&recovery);
                        step_dispatches.extend(recovery);
                    }
                }
                guardrail_context =
                    build_guardrail_context(&guardrail, &step_dispatches).or(guardrail_context);
                last_error = Some(guardrail.to_string());
                last_dispatches = step_dispatches;
                if guardrail.should_abort_retry() {
                    break 'attempts;
                }
                continue;
            }
            let summary = success_summary(step, &step_dispatches);
            let (total_wait_ms, total_run_ms) = aggregate_dispatch_totals(&step_dispatches);
            let dispatches = step_dispatches;
            runtime_state.clear_auto_act_state(&step.id);
            let report = StepExecutionReport {
                step_id: step.id.clone(),
                title: step.title.clone(),
                tool_kind: tool_kind.clone(),
                status: StepExecutionStatus::Success,
                attempts,
                error: None,
                dispatches,
                total_wait_ms,
                total_run_ms,
                observation_summary: summary,
                blocker_kind: None,
                agent_state: agent_state_metadata(step),
            };
            record_step_metrics(&report);
            return report;
        }

        if let Some(plan) = auto_act_refresh_plan(step) {
            if should_trigger_auto_act_refresh(step, last_error.as_deref(), &step_dispatches) {
                if let Some(dispatches) = attempt_auto_act_refresh(
                    dispatcher,
                    &task_id,
                    &plan,
                    routing_hint,
                    options,
                    runtime_state,
                    status_handle,
                )
                .await
                {
                    step_dispatches.extend(dispatches);
                    last_error = None;
                    guardrail_context = None;
                    continue 'attempts;
                }
            }
        }

        debug!(step = %step.id, attempt, "Retrying plan step after failure");
        last_dispatches = step_dispatches;
    }

    let (total_wait_ms, total_run_ms) = aggregate_dispatch_totals(&last_dispatches);
    let summary = guardrail_context
        .as_ref()
        .and_then(|ctx| ctx.summary.clone())
        .or_else(|| failure_summary(step, &last_dispatches, last_error.as_deref()));
    let blocker = guardrail_context
        .as_ref()
        .and_then(|ctx| ctx.blocker_kind.clone())
        .or_else(|| failure_blocker(step, last_error.as_deref()));
    runtime_state.clear_auto_act_state(&step.id);
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
        observation_summary: summary,
        blocker_kind: blocker,
        agent_state: agent_state_metadata(step),
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

// Use routing_hint_from_exec instead - removed duplicate function

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
        routing_hint: Some(routing_hint_from_exec(route)),
    };

    match dispatch_once(dispatcher, request).await {
        Ok((action_id, output)) => {
            let (normalized_output, artifacts) =
                normalize_dispatch_output("preview", output.output.clone());
            let error = output.error.as_ref().map(|err| err.to_string());
            Some(dispatch_record_from_output(
                "preview".to_string(),
                &action_id,
                &output,
                normalized_output,
                artifacts,
                error,
            ))
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

async fn capture_manual_observation<D: Dispatcher + ?Sized>(
    dispatcher: &D,
    task_id: &TaskId,
    route: &ExecRoute,
    options: &FlowExecutionOptions,
) -> Result<Option<DispatchRecord>> {
    let call_options = CallOptions {
        timeout: Duration::from_secs(15),
        priority: options.priority,
        interruptible: true,
        retry: RetryOpt {
            max: 0,
            backoff: Duration::from_millis(200),
        },
    };

    let payload = json!({
        "reason": "manual_resume",
        "task_id": task_id.0,
    });

    let tool_call = ToolCall {
        call_id: Some(format!("{}::manual-observe", task_id.0)),
        task_id: Some(task_id.clone()),
        tool: OBSERVATION_CANONICAL.to_string(),
        payload,
    };

    let hint = routing_hint_from_exec(route);
    let request = DispatchRequest {
        tool_call,
        options: call_options,
        routing_hint: Some(hint),
    };

    match dispatch_once(dispatcher, request).await {
        Ok((action_id, output)) => {
            let (normalized_output, artifacts) =
                normalize_dispatch_output("manual_observe", output.output.clone());
            let error = output.error.as_ref().map(|err| err.to_string());
            let record = dispatch_record_from_output(
                "manual-observe".to_string(),
                &action_id,
                &output,
                normalized_output,
                artifacts,
                error,
            );
            Ok(Some(record))
        }
        Err(err) => {
            warn!(task = %task_id.0, ?err, "manual observation dispatch failed");
            Ok(None)
        }
    }
}

fn exec_route_from_context(route: &ManualRouteContext) -> Option<ExecRoute> {
    let session = SessionId(route.session.clone());
    let page_value = route.page.as_ref()?.clone();
    let frame = route
        .frame
        .as_ref()
        .map(|value| FrameId(value.clone()))
        .unwrap_or_else(FrameId::new);
    Some(ExecRoute::new(session, PageId(page_value), frame))
}

fn routing_hint_from_exec(route: &ExecRoute) -> RoutingHint {
    RoutingHint {
        session: Some(route.session.clone()),
        page: Some(route.page.clone()),
        frame: Some(route.frame.clone()),
        prefer: Some(RoutePrefer::Focused),
    }
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
    current_search_context: Option<SearchContext>,
    memory_entries: Vec<ExecutionMemoryEntry>,
    preferred_route: Option<ExecRoute>,
    auto_act_refreshes: u32,
    auto_act_exclusions: HashMap<String, Vec<String>>,
}

#[derive(Clone, Debug)]
struct WeatherRouteRecord {
    destination_url: String,
}

#[derive(Clone, Debug, Default)]
struct GuardrailContext {
    summary: Option<String>,
    blocker_kind: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ExecutionMemoryEntry {
    pub step_id: String,
    pub title: String,
    pub observation_summary: Option<String>,
    pub blocker_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_goal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_context: Option<SearchContext>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SearchContext {
    pub query: Option<String>,
    pub engine: Option<String>,
    pub results_selector: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone, Debug)]
struct AutoActRefreshPlan {
    engine: String,
    queries: Vec<String>,
    max_retries: u32,
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
            self.record_route_hint(&dispatch.route);
        }
    }

    fn absorb_step_result(&mut self, step: &AgentPlanStep, report: &StepExecutionReport) {
        if matches!(
            step.tool.kind,
            AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case("weather.search")
        ) {
            self.record_weather_dispatches(&report.dispatches);
        } else if matches!(
            step.tool.kind,
            AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case("browser.search")
        ) {
            self.record_search_context(report);
        }
        self.record_memory_entry(step, report);
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
        for dispatch in dispatches {
            self.record_route_hint(&dispatch.route);
        }
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

    fn record_search_context(&mut self, report: &StepExecutionReport) {
        let dispatch = report
            .dispatches
            .iter()
            .find(|dispatch| dispatch.label == "action");
        let Some(dispatch) = dispatch else {
            return;
        };
        let Some(payload) = dispatch_payload(dispatch) else {
            return;
        };
        self.store_search_context(payload);
    }

    fn record_refresh_search_context(&mut self, dispatch: &DispatchRecord) {
        let Some(payload) = dispatch_payload(dispatch) else {
            return;
        };
        self.store_search_context(payload);
    }

    fn store_search_context(&mut self, payload: &Value) {
        if let Some(context) = search_context_from_payload(payload) {
            self.current_search_context = Some(context);
        }
    }

    fn record_memory_entry(&mut self, step: &AgentPlanStep, report: &StepExecutionReport) {
        let agent_state = report.agent_state.as_ref();
        if agent_state.is_none()
            && report.observation_summary.is_none()
            && report.blocker_kind.is_none()
        {
            return;
        }
        let search_context = if matches!(
            step.tool.kind,
            AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case("browser.search")
        ) {
            self.current_search_context.clone()
        } else {
            None
        };
        let entry = ExecutionMemoryEntry {
            step_id: report.step_id.clone(),
            title: step.title.clone(),
            observation_summary: report.observation_summary.clone(),
            blocker_kind: report.blocker_kind.clone(),
            thinking: agent_state_text(agent_state, "thinking"),
            evaluation: agent_state_text(agent_state, "evaluation"),
            memory: agent_state_text(agent_state, "memory"),
            next_goal: agent_state_text(agent_state, "next_goal"),
            search_context,
        };
        self.memory_entries.push(entry);
    }

    fn record_auto_act_refresh(&mut self) {
        self.auto_act_refreshes = self.auto_act_refreshes.saturating_add(1);
    }

    fn auto_act_refresh_count(&self) -> u32 {
        self.auto_act_refreshes
    }

    fn memory_log(&self) -> Vec<ExecutionMemoryEntry> {
        self.memory_entries.clone()
    }

    fn record_route_hint(&mut self, route: &ExecRoute) {
        self.preferred_route = Some(route.clone());
    }

    fn resolve_routing_hint(&self, fallback: &Option<RoutingHint>) -> Option<RoutingHint> {
        if let Some(route) = &self.preferred_route {
            return Some(routing_hint_from_exec(route));
        }
        fallback.clone()
    }

    fn apply_auto_act_exclusions(&self, step: &AgentPlanStep, specs: &mut [DispatchSpec]) {
        if !is_auto_act_step(step) {
            return;
        }
        let Some(spec) = specs
            .iter_mut()
            .find(|spec| spec.label == "action" && spec.tool == "browser.search.click-result")
        else {
            return;
        };
        let Some(payload) = spec.payload.as_object_mut() else {
            return;
        };
        if let Some(urls) = self.auto_act_exclusions.get(&step.id) {
            let filtered: Vec<Value> = urls
                .iter()
                .map(|url| url.trim())
                .filter(|url| !url.is_empty())
                .map(|url| Value::String(url.to_string()))
                .collect();
            if filtered.is_empty() {
                payload.remove("exclude_urls");
            } else {
                payload.insert("exclude_urls".to_string(), Value::Array(filtered));
            }
        } else {
            payload.remove("exclude_urls");
        }
    }

    fn record_auto_act_exclusions(&mut self, step: &AgentPlanStep, record: &DispatchRecord) {
        if !is_auto_act_step(step) || record.label != "action" {
            return;
        }
        if let Some(urls) = auto_act_exclusions_from_output(record) {
            if urls.is_empty() {
                self.auto_act_exclusions.remove(&step.id);
            } else {
                self.auto_act_exclusions.insert(step.id.clone(), urls);
            }
        }
    }

    fn clear_auto_act_state(&mut self, step_id: &str) {
        self.auto_act_exclusions.remove(step_id);
    }
}

/// Check if step uses a custom tool with given name (case-insensitive)
fn step_has_custom_tool(step: &AgentPlanStep, tool_name: &str) -> bool {
    matches!(
        step.tool.kind,
        AgentToolKind::Custom { ref name, .. } if name.eq_ignore_ascii_case(tool_name)
    )
}

fn is_auto_act_step(step: &AgentPlanStep) -> bool {
    step_has_custom_tool(step, "browser.search.click-result")
}

fn should_trigger_auto_act_refresh(
    step: &AgentPlanStep,
    last_error: Option<&str>,
    dispatches: &[DispatchRecord],
) -> bool {
    if !is_auto_act_step(step) {
        return false;
    }
    if last_error
        .map(auto_act_error_requires_refresh)
        .unwrap_or(false)
    {
        return true;
    }
    dispatches.iter().any(|record| {
        record.label == "action"
            && record
                .error
                .as_deref()
                .map(auto_act_error_requires_refresh)
                .unwrap_or(false)
    })
}

fn auto_act_error_requires_refresh(message: &str) -> bool {
    message.contains("[auto_act_candidates_exhausted]")
        || message.contains("timed out")
        || message.contains("WaitTimeout")
}

fn auto_act_exclusions_from_output(record: &DispatchRecord) -> Option<Vec<String>> {
    let root = record.output.as_ref()?.as_object()?;
    let payload = root.get("output")?.as_object()?;
    let urls = payload.get("excluded_urls")?.as_array()?;
    let collected: Vec<String> = urls
        .iter()
        .filter_map(Value::as_str)
        .map(|url| url.trim())
        .filter(|url| !url.is_empty())
        .map(|url| url.to_string())
        .collect();
    Some(collected)
}

fn search_context_from_payload(payload: &Value) -> Option<SearchContext> {
    let Some(object) = payload.as_object() else {
        return None;
    };
    let target = object
        .get("output")
        .and_then(Value::as_object)
        .unwrap_or(object);
    let query = target
        .get("query")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let engine = target
        .get("engine")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let results_selector = target
        .get("results_selector")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let url = target
        .get("url")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    if query.is_none() && engine.is_none() && results_selector.is_none() && url.is_none() {
        return None;
    }
    Some(SearchContext {
        query,
        engine,
        results_selector,
        url,
    })
}

fn agent_state_text(state: Option<&Value>, key: &str) -> Option<String> {
    let map = state?.as_object()?;
    map.get(key)
        .and_then(Value::as_str)
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .map(|text| text.to_string())
}

struct ManualOverrideController {
    manager: Arc<ManualSessionManager>,
    status: Option<TaskStatusHandle>,
    task_id: TaskId,
}

impl ManualOverrideController {
    fn new(
        manager: Arc<ManualSessionManager>,
        status: Option<TaskStatusHandle>,
        task_id: TaskId,
    ) -> Self {
        Self {
            manager,
            status,
            task_id,
        }
    }

    async fn wait_if_active<D: Dispatcher + ?Sized>(
        &self,
        dispatcher: &D,
        options: &FlowExecutionOptions,
    ) -> Result<()> {
        let Some(mut snapshot) = self.manager.snapshot(&self.task_id) else {
            return Ok(());
        };

        if matches!(snapshot.status, ManualOverridePhase::Requested) {
            if let Some(updated) = self
                .manager
                .set_phase(&self.task_id, ManualOverridePhase::Active)
            {
                snapshot = updated;
            }
        }

        let Some(route) = snapshot.route.clone() else {
            warn!(task = %self.task_id.0, "manual override missing route context");
            return Ok(());
        };

        self.update_status(snapshot.clone());
        let paused_at = std::time::Instant::now();
        while self.manager.snapshot(&self.task_id).is_some() {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        self.handle_resume(dispatcher, options, &route, paused_at.elapsed())
            .await?;
        self.clear_status();
        Ok(())
    }

    fn update_status(&self, snapshot: ManualOverrideSnapshot) {
        if let Some(handle) = &self.status {
            handle.update_manual_override(snapshot);
            handle.log(
                TaskLogLevel::Info,
                "Manual takeover active; automation paused",
            );
        }
    }

    fn clear_status(&self) {
        if let Some(handle) = &self.status {
            handle.clear_manual_override();
        }
    }

    async fn handle_resume<D: Dispatcher + ?Sized>(
        &self,
        dispatcher: &D,
        options: &FlowExecutionOptions,
        route_ctx: &ManualRouteContext,
        paused_for: Duration,
    ) -> Result<()> {
        let Some(exec_route) = exec_route_from_context(route_ctx) else {
            warn!(task = %self.task_id.0, "manual override resume missing exec route context");
            return Ok(());
        };

        let mut evidences = Vec::new();
        let mut dispatch_index = 0usize;
        if let Some(record) =
            capture_manual_observation(dispatcher, &self.task_id, &exec_route, options).await?
        {
            let values = dispatch_artifact_values("manual_resume", dispatch_index, &record);
            evidences.extend(values);
            dispatch_index += 1;
        }

        if let Some(record) = capture_preview_frame(
            dispatcher,
            &self.task_id,
            "manual_resume",
            &exec_route,
            options,
        )
        .await
        {
            let values = dispatch_artifact_values("manual_resume", dispatch_index, &record);
            evidences.extend(values);
        }

        if let Some(handle) = &self.status {
            if !evidences.is_empty() {
                handle.push_evidence(&evidences);
            }
            let step_index = handle
                .snapshot()
                .and_then(|snap| snap.current_step)
                .unwrap_or(0);
            handle.push_agent_history(AgentHistoryEntry {
                timestamp: Utc::now(),
                step_index,
                step_id: format!("manual_resume_{}", step_index),
                title: "Manual override resumed".to_string(),
                status: AgentHistoryStatus::Success,
                attempts: 1,
                message: Some(format!(
                    "User manually controlled the browser for {:.1} seconds",
                    paused_for.as_secs_f32()
                )),
                observation_summary: Some(
                    "Captured DOM snapshot and screenshot after manual takeover".to_string(),
                ),
                obstruction: None,
                structured_summary: None,
                thinking: None,
                evaluation: None,
                memory: None,
                next_goal: None,
                search_context: None,
                tool_kind: Some("manual_override".to_string()),
                wait_ms: None,
                run_ms: None,
            });
            handle.log(
                TaskLogLevel::Info,
                "Manual takeover resumed; automation continuing",
            );
        }

        Ok(())
    }
}

fn is_note_step(step: &AgentPlanStep) -> bool {
    matches!(
        step.tool.kind,
        AgentToolKind::Custom { ref name, .. } if is_note_tool_name(name)
    )
}

fn is_note_tool_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("agent.note")
        || name.eq_ignore_ascii_case("agent.evaluate")
        || name.to_ascii_lowercase().ends_with("note")
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
        observation_summary: Some(note_text.clone()),
        blocker_kind: None,
        agent_state: agent_state_metadata(step),
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

fn dispatch_record_from_output(
    label: String,
    action_id: &ActionId,
    output: &DispatchOutput,
    normalized_output: Option<Value>,
    artifacts: Vec<RunArtifact>,
    error: Option<String>,
) -> DispatchRecord {
    let (wait_ms, run_ms) = timeline_metrics(&output.timeline);
    DispatchRecord {
        label,
        action_id: action_id.0.clone(),
        route: output.route.clone(),
        wait_ms,
        run_ms,
        output: normalized_output,
        artifacts,
        error,
    }
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

fn tool_failure_from_output(dispatch: &DispatchRecord) -> Option<String> {
    let root = dispatch.output.as_ref()?.as_object()?;
    let success = root.get("success").and_then(Value::as_bool).unwrap_or(true);
    if success {
        return None;
    }
    if let Some(err) = root.get("error").and_then(Value::as_str) {
        if !err.trim().is_empty() {
            return Some(err.to_string());
        }
    }
    Some("tool reported failure".to_string())
}

fn auto_act_attempt_state(record: &DispatchRecord) -> Option<Value> {
    let root = record.output.as_ref()?.as_object()?;
    let payload = root.get("output")?.as_object()?;
    let status = payload.get("status")?.as_str()?;
    if status != "auto_act_candidates_exhausted" {
        return None;
    }
    let attempts = payload.get("attempts")?.clone();
    let attempt_count = attempts.as_array().map(|arr| arr.len()).unwrap_or_default();
    let excluded = payload
        .get("excluded_urls")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let excluded_count = excluded.as_array().map(|arr| arr.len()).unwrap_or_default();
    Some(json!({
        "history_items": [
            {
                "tag": "auto_act_attempt",
                "label": format!("候选耗尽 {} 次", attempt_count),
                "content": format!("排除 {} 个链接，准备刷新搜索", excluded_count),
                "recorded_at": Utc::now().to_rfc3339(),
                "attempts": attempts,
                "excluded_urls": excluded,
            }
        ]
    }))
}

fn emit_auto_act_state(handle: &TaskStatusHandle, record: &DispatchRecord) {
    if let Some(state) = auto_act_attempt_state(record) {
        handle.set_message_state(Some(state));
    }
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

async fn attempt_auto_act_refresh<D: Dispatcher + ?Sized>(
    dispatcher: &D,
    task_id: &TaskId,
    plan: &AutoActRefreshPlan,
    routing_hint: &Option<RoutingHint>,
    options: &FlowExecutionOptions,
    runtime_state: &mut FlowRuntimeState,
    status_handle: Option<&TaskStatusHandle>,
) -> Option<Vec<DispatchRecord>> {
    if runtime_state.auto_act_refresh_count() >= plan.max_retries {
        return None;
    }
    runtime_state.record_auto_act_refresh();
    let attempt = runtime_state.auto_act_refresh_count();
    let query = plan
        .queries
        .get((attempt.saturating_sub(1)) as usize)
        .cloned()
        .or_else(|| plan.queries.last().cloned())?;
    if let Some(handle) = status_handle {
        handle.log(
            TaskLogLevel::Info,
            format!(
                "AutoAct guardrail刷新 {}/{}：{}",
                attempt, plan.max_retries, query
            ),
        );
        let overlay = json!([{
            "kind": "stage_timeline",
            "deterministic": false,
            "stages": [
                {
                    "stage": "auto_act",
                    "label": "AutoAct",
                    "status": "retrying",
                    "strategy": plan.engine,
                    "detail": format!(
                        "候选项耗尽，准备第 {}/{} 次 Guardrail 刷新",
                        attempt, plan.max_retries
                    ),
                },
                {
                    "stage": "guardrail",
                    "label": "Guardrail 刷新",
                    "status": "running",
                    "strategy": plan.engine,
                    "detail": format!("重新搜索：{}", query),
                },
                {
                    "stage": "retry",
                    "label": "Retry",
                    "status": "pending",
                    "strategy": "AutoAct",
                    "detail": "刷新完成后继续点击候选",
                }
            ]
        }]);
        handle.push_execution_overlays(overlay);
    }
    let mut payload = json!({
        "query": query,
        "engine": plan.engine,
    });
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "auto_act_refresh_attempt".to_string(),
            Value::Number(attempt.into()),
        );
    }
    let call_options = CallOptions {
        timeout: Duration::from_millis(30_000),
        priority: options.priority,
        interruptible: true,
        retry: RetryOpt {
            max: 0,
            backoff: Duration::from_millis(200),
        },
    };
    let tool_call = ToolCall {
        call_id: Some(format!("{}::auto-act-refresh-{}", task_id.0, attempt)),
        task_id: Some(task_id.clone()),
        tool: "browser.search".to_string(),
        payload,
    };
    let request = DispatchRequest {
        tool_call,
        options: call_options,
        routing_hint: runtime_state.resolve_routing_hint(routing_hint),
    };
    match dispatch_once(dispatcher, request).await {
        Ok((action_id, output)) => {
            if let Some(err) = output.error.as_ref() {
                warn!(
                    task = %task_id.0,
                    error = %err,
                    "AutoAct guardrail refresh search failed"
                );
                return None;
            }
            let (normalized_output, artifacts) =
                normalize_dispatch_output("auto_act-refresh", output.output.clone());
            runtime_state.record_route_hint(&output.route);
            let record = dispatch_record_from_output(
                "auto_act-refresh".to_string(),
                &action_id,
                &output,
                normalized_output,
                artifacts,
                None,
            );
            runtime_state.record_refresh_search_context(&record);
            Some(vec![record])
        }
        Err(err) => {
            warn!(task = %task_id.0, ?err, "AutoAct guardrail refresh dispatch failed");
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
        AgentToolKind::Custom { ref name, .. }
            if name.eq_ignore_ascii_case(OBSERVATION_CANONICAL)
                || name.eq_ignore_ascii_case("market.quote.fetch")
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

    fn blocker_kind(&self) -> Option<&'static str> {
        match self {
            ObservationGuardrail::UrlMismatch { .. } => Some("url_mismatch"),
            ObservationGuardrail::WeatherBaiduHome { .. } => Some("weather_results_missing"),
            ObservationGuardrail::Blocked { reason } => {
                let lower = reason.to_ascii_lowercase();
                if lower.contains("404") {
                    Some("page_not_found")
                } else if lower.contains("403") {
                    Some("access_blocked")
                } else if lower.contains("captcha") {
                    Some("captcha_block")
                } else if lower.contains("verification") {
                    Some("verification_required")
                } else if lower.contains("too many requests") || lower.contains("访问过于频繁")
                {
                    Some("rate_limited")
                } else {
                    Some("observation_blocked")
                }
            }
        }
    }

    fn should_abort_retry(&self) -> bool {
        match self {
            ObservationGuardrail::UrlMismatch { .. } => true,
            ObservationGuardrail::WeatherBaiduHome { .. } => false,
            ObservationGuardrail::Blocked { reason } => {
                let lower = reason.to_ascii_lowercase();
                lower.contains("404") || lower.contains("notfound")
            }
        }
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

fn guardrail_observation_summary(dispatches: &[DispatchRecord]) -> Option<String> {
    let observation = observation_snapshot_from_dispatches(dispatches)?;
    summarize_observation(observation)
}

fn success_summary(step: &AgentPlanStep, dispatches: &[DispatchRecord]) -> Option<String> {
    if is_note_step(step) {
        return Some(extract_note_message(step));
    }
    if is_observation_step(step) {
        return guardrail_observation_summary(dispatches);
    }
    match &step.tool.kind {
        AgentToolKind::Navigate { .. } => navigation_summary(dispatches),
        AgentToolKind::Scroll { .. } => detail_summary(step, "滚动页面"),
        AgentToolKind::Click { .. } => detail_summary(step, "执行点击操作"),
        AgentToolKind::TypeText { .. } => detail_summary(step, "输入文本"),
        AgentToolKind::Select { .. } => detail_summary(step, "选择页面元素"),
        AgentToolKind::Wait { .. } => detail_summary(step, "等待页面状态"),
        AgentToolKind::Custom { name, .. } if name.eq_ignore_ascii_case("agent.evaluate") => {
            detail_summary(step, "评估页面状态")
        }
        _ => None,
    }
}

fn agent_state_metadata(step: &AgentPlanStep) -> Option<Value> {
    step.metadata.get("agent_state").cloned()
}

fn detail_summary(step: &AgentPlanStep, fallback: &str) -> Option<String> {
    let trimmed = step.detail.trim();
    if trimmed.is_empty() {
        Some(fallback.to_string())
    } else {
        Some(trimmed.to_string())
    }
}

fn navigation_summary(dispatches: &[DispatchRecord]) -> Option<String> {
    let payload = first_action_payload(dispatches)?;
    let url = payload.get("url").and_then(Value::as_str)?;
    Some(format!("已导航至 {}", url))
}

fn first_action_payload(dispatches: &[DispatchRecord]) -> Option<&Value> {
    dispatches
        .iter()
        .find(|dispatch| dispatch.label == "action")
        .and_then(dispatch_payload)
}

fn failure_summary(
    step: &AgentPlanStep,
    dispatches: &[DispatchRecord],
    last_error: Option<&str>,
) -> Option<String> {
    let AgentToolKind::Custom { name, payload } = &step.tool.kind else {
        return None;
    };
    if name.eq_ignore_ascii_case("market.quote.fetch") {
        let detail = last_error
            .filter(|msg| !msg.trim().is_empty())
            .unwrap_or("报价采集失败");
        return Some(format!("行情采集失败：{}", detail));
    } else if name.eq_ignore_ascii_case("browser.search") {
        let selector = dispatches
            .iter()
            .find(|dispatch| dispatch.label == "action")
            .and_then(dispatch_payload)
            .and_then(|payload| {
                payload
                    .get("output")
                    .and_then(|value| value.get("results_selector"))
                    .and_then(Value::as_str)
            });
        let detail = last_error
            .filter(|msg| !msg.trim().is_empty())
            .unwrap_or("搜索结果未出现");
        if let Some(selector) = selector {
            return Some(format!(
                "浏览器搜索失败：{}，检查结果容器 {} 是否渲染",
                detail, selector
            ));
        }
        return Some(format!("浏览器搜索失败：{}", detail));
    } else if name.eq_ignore_ascii_case("browser.search.click-result") {
        let detail = last_error
            .filter(|msg| !msg.trim().is_empty())
            .unwrap_or("未能打开权威搜索结果");
        let domains = payload
            .get("domains")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if domains.is_empty() {
            return Some(format!("搜索结果点击失败：{}", detail));
        }
        return Some(format!(
            "未能打开权威站点 {}：{}",
            domains.join(" / "),
            detail
        ));
    } else if name.eq_ignore_ascii_case("browser.close-modal") {
        let detail = last_error
            .filter(|msg| !msg.trim().is_empty())
            .unwrap_or("未能关闭弹窗");
        return Some(format!("关闭弹窗失败：{}", detail));
    } else if name.eq_ignore_ascii_case("data.validate-target") {
        let code = validation_error_code(last_error);
        let detail = validation_error_detail(last_error).unwrap_or("目标页面校验失败");
        let (keywords, domains, expected_status) = validation_step_context(step);
        let summary = match code {
            Some("target_validation_blocked") => {
                "目标页面无法访问或被拦截，请改用可靠行情源".to_string()
            }
            Some("target_validation_keywords_missing") => {
                if keywords.is_empty() {
                    format!("目标校验失败：{}", detail)
                } else {
                    format!("目标校验失败：页面缺少关键词 {}", keywords.join("/"),)
                }
            }
            Some("target_validation_domain_mismatch") => {
                if domains.is_empty() {
                    format!("目标校验失败：{}", detail)
                } else {
                    format!("目标校验失败：页面域名不在 {}", domains.join("/"),)
                }
            }
            Some("target_validation_status_mismatch") => {
                if let Some(status) = expected_status {
                    format!("目标校验失败：HTTP 状态与期望 {} 不符", status)
                } else {
                    format!("目标校验失败：{}", detail)
                }
            }
            _ => format!("目标校验失败：{}", detail),
        };
        return Some(summary);
    }
    None
}

fn failure_blocker(step: &AgentPlanStep, last_error: Option<&str>) -> Option<String> {
    let AgentToolKind::Custom { name, .. } = &step.tool.kind else {
        return None;
    };
    let lowered = name.to_ascii_lowercase();
    match lowered.as_str() {
        "market.quote.fetch" => {
            if last_error
                .map(|msg| msg.contains("报价采集失败"))
                .unwrap_or(true)
            {
                Some("quote_fetch_failed".to_string())
            } else {
                None
            }
        }
        "browser.search" | "browser.search.click-result" => Some("search_no_results".to_string()),
        "browser.close-modal" => {
            if last_error
                .map(|msg| msg.contains("未找到可关闭的弹窗"))
                .unwrap_or(false)
            {
                Some("popup_unclosed".to_string())
            } else {
                None
            }
        }
        "data.validate-target" => match validation_error_code(last_error) {
            Some("target_validation_blocked") | Some("target_validation_status_mismatch") => {
                Some("page_not_found".to_string())
            }
            Some("target_validation_domain_mismatch") => Some("url_mismatch".to_string()),
            Some("target_validation_keywords_missing") => {
                Some("target_keywords_missing".to_string())
            }
            _ => Some("target_validation_failed".to_string()),
        },
        _ => None,
    }
}

fn validation_error_code(message: Option<&str>) -> Option<&str> {
    let text = message?;
    let rest = text.strip_prefix('[')?;
    let end = rest.find(']')?;
    Some(&rest[..end])
}

fn validation_error_detail(message: Option<&str>) -> Option<&str> {
    let text = message?;
    if let Some(end) = text.find(']') {
        let detail = text[end + 1..].trim();
        if detail.is_empty() {
            None
        } else {
            Some(detail)
        }
    } else {
        Some(text)
    }
}

fn validation_step_context(step: &AgentPlanStep) -> (Vec<String>, Vec<String>, Option<u16>) {
    if let AgentToolKind::Custom { payload, .. } = &step.tool.kind {
        let keywords = payload
            .get("keywords")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let domains = payload
            .get("allowed_domains")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let expected = payload
            .get("expected_status")
            .and_then(Value::as_u64)
            .map(|value| value as u16);
        (keywords, domains, expected)
    } else {
        (Vec::new(), Vec::new(), None)
    }
}

fn summarize_observation(observation: &Value) -> Option<String> {
    let title = observation_primary_title(observation);
    let snippet = observation_text_snippet(observation);
    if title.is_empty() && snippet.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if !title.is_empty() {
        parts.push(format!("《{}》", title));
    }
    if !snippet.is_empty() {
        parts.push(truncate_text(&snippet, 200));
    }
    Some(parts.join(": "))
}

fn observation_block_reason(observation: &Value, actual_url: &str) -> Option<String> {
    let title = observation_primary_title(observation);
    let text = observation_text_snippet(observation);
    if title.is_empty() && text.is_empty() {
        return None;
    }
    detect_block_reason(&title, &text, Some(actual_url))
}

fn build_guardrail_context(
    guardrail: &ObservationGuardrail,
    dispatches: &[DispatchRecord],
) -> Option<GuardrailContext> {
    let summary = guardrail_observation_summary(dispatches);
    let blocker_kind = guardrail.blocker_kind().map(|value| value.to_string());
    if summary.is_none() && blocker_kind.is_none() {
        None
    } else {
        Some(GuardrailContext {
            summary,
            blocker_kind,
        })
    }
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
    step_has_custom_tool(step, "data.parse.weather")
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

    fn auto_act_step() -> AgentPlanStep {
        AgentPlanStep::new(
            "act-1",
            "定位搜索结果",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "browser.search.click-result".to_string(),
                    payload: json!({}),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(60_000),
            },
        )
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
    fn auto_act_refresh_triggers_on_timeout_error() {
        let step = auto_act_step();
        let dispatch = DispatchRecord {
            label: "action".to_string(),
            action_id: ActionId::new().0,
            route: synthetic_route(),
            wait_ms: 0,
            run_ms: 0,
            output: None,
            artifacts: Vec::new(),
            error: Some("tool browser.search.click-result timed out after 60s".to_string()),
        };
        assert!(should_trigger_auto_act_refresh(
            &step,
            Some("tool browser.search.click-result timed out after 60s"),
            &[dispatch]
        ));
    }

    #[test]
    fn auto_act_refresh_ignores_non_auto_steps() {
        let step = observation_step(None);
        let dispatch = DispatchRecord {
            label: "action".to_string(),
            action_id: ActionId::new().0,
            route: synthetic_route(),
            wait_ms: 0,
            run_ms: 0,
            output: None,
            artifacts: Vec::new(),
            error: Some("tool browser.search.click-result timed out after 60s".to_string()),
        };
        assert!(!should_trigger_auto_act_refresh(
            &step,
            Some("tool browser.search.click-result timed out after 60s"),
            &[dispatch]
        ));
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
    fn guardrail_abort_retry_only_for_fatal_blockers() {
        let fatal = ObservationGuardrail::Blocked {
            reason: "Page reports 404/NotFound message".to_string(),
        };
        assert!(fatal.should_abort_retry());

        let mismatch = ObservationGuardrail::UrlMismatch {
            expected: "https://quote.eastmoney.com".to_string(),
            actual: "https://example.com".to_string(),
        };
        assert!(mismatch.should_abort_retry());

        let weather = ObservationGuardrail::WeatherBaiduHome {
            actual: "https://www.baidu.com".to_string(),
        };
        assert!(!weather.should_abort_retry());
    }

    #[test]
    fn runtime_state_prefers_updated_route_hint() {
        let mut state = FlowRuntimeState::default();
        let fallback = Some(RoutingHint {
            session: Some(SessionId::new()),
            page: Some(PageId::new()),
            frame: Some(FrameId::new()),
            prefer: Some(RoutePrefer::Focused),
        });
        let initial = state
            .resolve_routing_hint(&fallback)
            .expect("fallback hint available");
        assert_eq!(initial.page, fallback.as_ref().unwrap().page);

        let route = ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new());
        state.record_route_hint(&route);
        let resolved = state.resolve_routing_hint(&fallback).expect("updated hint");
        assert_eq!(resolved.page, Some(route.page.clone()));
        assert_eq!(resolved.session, Some(route.session.clone()));
    }

    #[test]
    fn runtime_state_emits_hint_without_fallback_after_route_recorded() {
        let mut state = FlowRuntimeState::default();
        assert!(state.resolve_routing_hint(&None).is_none());
        let route = ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new());
        state.record_route_hint(&route);
        let resolved = state.resolve_routing_hint(&None).expect("hint from route");
        assert_eq!(resolved.page, Some(route.page));
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

    #[test]
    fn failure_helpers_mark_quote_fetch() {
        let step = AgentPlanStep::new(
            "step-q",
            "采集行情",
            AgentTool {
                kind: AgentToolKind::Custom {
                    name: "market.quote.fetch".to_string(),
                    payload: json!({}),
                },
                wait: WaitMode::None,
                timeout_ms: Some(1_000),
            },
        );
        let summary =
            failure_summary(&step, &[], Some("报价采集失败：所有数据源均不可用")).expect("summary");
        assert!(summary.contains("行情采集失败"));
        let blocker = failure_blocker(&step, Some("报价采集失败"));
        assert_eq!(blocker.as_deref(), Some("quote_fetch_failed"));
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
                || normalized == "market.quote.fetch"
                || normalized.starts_with("data.validate.")
                || normalized.eq_ignore_ascii_case("data.validate-target")
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
            } else if normalized.eq_ignore_ascii_case("browser.search") {
                Ok((
                    "browser.search".to_string(),
                    merge_custom_payload(payload, task_id),
                ))
            } else if normalized.eq_ignore_ascii_case("browser.search.click-result") {
                Ok((
                    "browser.search.click-result".to_string(),
                    merge_custom_payload(payload, task_id),
                ))
            } else if normalized.eq_ignore_ascii_case("browser.close-modal") {
                Ok((
                    "browser.close-modal".to_string(),
                    merge_custom_payload(payload, task_id),
                ))
            } else if normalized.eq_ignore_ascii_case("browser.send-esc") {
                Ok((
                    "browser.send-esc".to_string(),
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
        AgentToolKind::Done { success, text } => {
            // Done action is used in agent loop mode to signal task completion.
            // In plan-execute mode, we convert it to a note-like custom action.
            Ok((
                "agent.done".to_string(),
                json!({
                    "success": success,
                    "text": text,
                }),
            ))
        }
    }
}

fn auto_act_refresh_plan(step: &AgentPlanStep) -> Option<AutoActRefreshPlan> {
    let metadata = step.metadata.get("auto_act_refresh")?;
    let object = metadata.as_object()?;
    let mut queries: Vec<String> = object
        .get("queries")
        .and_then(|value| value.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if queries.is_empty() {
        if let Some(query) = object.get("query").and_then(Value::as_str) {
            let trimmed = query.trim();
            if !trimmed.is_empty() {
                queries.push(trimmed.to_string());
            }
        }
    }
    if queries.is_empty() {
        return None;
    }
    let engine = object
        .get("engine")
        .and_then(Value::as_str)
        .unwrap_or("baidu")
        .to_string();
    let max_retries = object
        .get("max_retries")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    if max_retries == 0 {
        return None;
    }
    Some(AutoActRefreshPlan {
        engine,
        queries,
        max_retries,
    })
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
        AgentToolKind::Done { .. } => "done",
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
