use std::net::SocketAddr;

use agent_core::AgentContext;
use axum::{extract::ConnectInfo, response::IntoResponse, routing::post, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{error, info, warn};

use crate::agent::{execute_plan, FlowExecutionOptions, FlowExecutionReport};
use crate::app_context::get_or_create_context;
use crate::intent::enrich_request_with_intent;
use crate::replan::augment_request_for_replan;
use crate::server::{rate_limit::RateLimitKind, ServeState};
use crate::task_store::{PersistedPlanRecord, PlanOriginMetadata, PlanSource, TaskPlanStore};
use crate::visualization::{
    build_execution_overlays, build_plan_overlays, execution_artifacts_from_report,
};
use crate::{
    agent_history_prompt, apply_blocker_hints, apply_perception_metadata, build_chat_runner,
    build_telemetry_payload, capture_chat_context_snapshot, flow_execution_report_payload,
    latest_observation_summary, latest_obstruction_kind, llm_cache_for_request,
    perception_snapshot_value, persist_execution_outputs, plan_payload,
    propagate_browser_state_metadata, LlmProviderConfig, LlmProviderSelection, PlannerSelection,
};

pub(crate) fn router() -> Router<ServeState> {
    Router::new().route("/api/chat", post(serve_chat_handler))
}

// Chat API structures
#[derive(Debug, Deserialize)]
struct ChatRequest {
    prompt: String,
    #[serde(default)]
    current_url: Option<String>,
    #[serde(default)]
    constraints: Option<Vec<String>>,
    #[serde(default)]
    execute: Option<bool>,
    #[serde(default)]
    planner: Option<String>,
    #[serde(default)]
    llm_provider: Option<String>,
    #[serde(default)]
    llm_model: Option<String>,
    #[serde(default)]
    llm_api_base: Option<String>,
    #[serde(default)]
    llm_temperature: Option<f32>,
    #[serde(default)]
    llm_api_key: Option<String>,
    #[serde(default)]
    llm_max_output_tokens: Option<u32>,
    #[serde(default)]
    max_replans: Option<u8>,
    #[serde(default)]
    capture_context: Option<bool>,
    #[serde(default)]
    context_timeout_secs: Option<u64>,
    #[serde(default)]
    context_screenshot: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ChatResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    flow: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifacts: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<serde_json::Value>,
    stdout: String,
    stderr: String,
}

async fn serve_chat_handler(
    axum::extract::State(state): axum::extract::State<ServeState>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    axum::Json(req): axum::Json<ChatRequest>,
) -> impl axum::response::IntoResponse {
    if !state
        .rate_limiter
        .allow(&client_addr.ip().to_string(), RateLimitKind::Chat)
    {
        let body = ChatResponse {
            success: false,
            plan: None,
            flow: None,
            artifacts: None,
            context: None,
            stdout: String::new(),
            stderr: "Too many requests".to_string(),
        };
        return (axum::http::StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response();
    }
    let ChatRequest {
        prompt,
        current_url,
        constraints,
        execute,
        planner,
        llm_provider,
        llm_model,
        llm_api_base,
        llm_temperature,
        llm_api_key,
        llm_max_output_tokens,
        max_replans,
        capture_context,
        context_timeout_secs,
        context_screenshot,
    } = req;

    let planner_choice = match planner.as_deref().map(PlannerSelection::from_str_case) {
        Some(Some(kind)) => kind,
        Some(None) => {
            let body = ChatResponse {
                success: false,
                plan: None,
                flow: None,
                artifacts: None,
                context: None,
                stdout: String::new(),
                stderr: "Unknown planner specified".to_string(),
            };
            return (axum::http::StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
        }
        None => PlannerSelection::Rule,
    };

    let llm_choice = match llm_provider
        .as_deref()
        .map(LlmProviderSelection::from_str_case)
    {
        Some(Some(kind)) => Some(kind),
        Some(None) => {
            let body = ChatResponse {
                success: false,
                plan: None,
                flow: None,
                artifacts: None,
                context: None,
                stdout: String::new(),
                stderr: "Unknown llm_provider specified".to_string(),
            };
            return (axum::http::StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
        }
        None => None,
    };

    let trimmed_prompt = prompt.trim();
    if trimmed_prompt.is_empty() {
        let body = ChatResponse {
            success: false,
            plan: None,
            flow: None,
            artifacts: None,
            context: None,
            stdout: String::new(),
            stderr: "Prompt must not be empty".to_string(),
        };
        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
    }
    if trimmed_prompt.len() > 2_000 {
        let body = ChatResponse {
            success: false,
            plan: None,
            flow: None,
            artifacts: None,
            context: None,
            stdout: String::new(),
            stderr: "Prompt exceeds 2000 characters".to_string(),
        };
        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
    }

    info!("chat request received");

    let (storage_path, policy_paths) = {
        let cfg = &*state.config;
        (Some(cfg.output_dir.clone()), cfg.policy_paths.clone())
    };
    let app_context = match get_or_create_context(
        state.tenant_id.clone(),
        storage_path.clone(),
        policy_paths.clone(),
    )
    .await
    {
        Ok(ctx) => ctx,
        Err(err) => {
            error!(?err, "failed to obtain app context for chat");
            let body = ChatResponse {
                success: false,
                plan: None,
                flow: None,
                artifacts: None,
                context: None,
                stdout: String::new(),
                stderr: format!("context error: {err}"),
            };
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(body),
            )
                .into_response();
        }
    };
    let plugin_registry = app_context.plugin_registry();

    let mut agent_context = AgentContext::default();
    if let Some(url) = current_url.clone() {
        agent_context.current_url = Some(url);
    }

    let mut context_snapshot: Option<Value> = None;
    let should_capture_context = capture_context.unwrap_or(agent_context.current_url.is_some());
    if should_capture_context {
        if let Some(url) = agent_context.current_url.clone() {
            match capture_chat_context_snapshot(
                &state,
                &url,
                context_screenshot.unwrap_or(true),
                context_timeout_secs,
            )
            .await
            {
                Ok(snapshot) => {
                    apply_perception_metadata(&mut agent_context, &snapshot);
                    context_snapshot = Some(perception_snapshot_value(&snapshot));
                }
                Err(err) => {
                    warn!(?err, "perception capture for chat failed");
                    agent_context.metadata.insert(
                        "perception_error".to_string(),
                        Value::String(err.to_string()),
                    );
                    context_snapshot = Some(json!({
                        "success": false,
                        "error": err.to_string(),
                    }));
                }
            }
        } else {
            warn!("capture_context requested but no current_url provided");
            context_snapshot = Some(json!({
                "success": false,
                "error": "current_url is required to capture context",
            }));
        }
    }

    let has_context = agent_context.session.is_some()
        || agent_context.page.is_some()
        || agent_context.current_url.is_some()
        || !agent_context.preferences.is_empty()
        || !agent_context.memory_hints.is_empty()
        || !agent_context.metadata.is_empty();

    let llm_model_for_origin = llm_model.clone();

    let cache_for_request = if planner_choice == PlannerSelection::Llm {
        llm_cache_for_request(&state, llm_choice, llm_model_for_origin.as_deref())
    } else {
        None
    };

    let llm_config = LlmProviderConfig {
        model: llm_model,
        api_base: llm_api_base,
        temperature: llm_temperature,
        api_key: llm_api_key,
        max_output_tokens: llm_max_output_tokens,
    };

    let runner = match build_chat_runner(
        planner_choice,
        llm_choice,
        llm_config,
        cache_for_request,
        Some(plugin_registry.clone()),
    ) {
        Ok(runner) => runner,
        Err(err) => {
            error!(?err, "failed to configure chat runner");
            let body = ChatResponse {
                success: false,
                plan: None,
                flow: None,
                artifacts: None,
                context: None,
                stdout: String::new(),
                stderr: "Failed to configure chat planner".to_string(),
            };
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(body),
            )
                .into_response();
        }
    };
    let constraint_list = constraints.unwrap_or_default();
    if constraint_list.len() > 10 {
        let body = ChatResponse {
            success: false,
            plan: None,
            flow: None,
            artifacts: None,
            context: None,
            stdout: String::new(),
            stderr: "Too many constraints (max 10)".to_string(),
        };
        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
    }
    if constraint_list.iter().any(|c| c.len() > 512) {
        let body = ChatResponse {
            success: false,
            plan: None,
            flow: None,
            artifacts: None,
            context: None,
            stdout: String::new(),
            stderr: "Constraint exceeds 512 characters".to_string(),
        };
        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
    }
    let mut agent_request = runner.request_from_prompt(
        trimmed_prompt.to_string(),
        if has_context {
            Some(agent_context)
        } else {
            None
        },
        constraint_list,
    );
    propagate_browser_state_metadata(&mut agent_request, context_snapshot.as_ref());
    enrich_request_with_intent(&mut agent_request, trimmed_prompt);

    let session = match runner.plan(agent_request.clone()).await {
        Ok(output) => output,
        Err(err) => {
            error!(?err, "chat planning failed");
            let body = ChatResponse {
                success: false,
                plan: None,
                flow: None,
                artifacts: None,
                context: context_snapshot.clone(),
                stdout: String::new(),
                stderr: format!("planner failed: {err}"),
            };
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(body),
            )
                .into_response();
        }
    };

    let mut current_session = session;
    let mut exec_request = agent_request.clone();
    let mut plan_history = vec![plan_payload(&current_session)];

    let plan_origin = PlanOriginMetadata {
        planner: planner_choice.label().to_string(),
        llm_provider: matches!(planner_choice, PlannerSelection::Llm)
            .then(|| llm_choice.unwrap_or_default().label().to_string()),
        llm_model: llm_model_for_origin.clone(),
    };

    match PersistedPlanRecord::from_session(
        &current_session,
        trimmed_prompt,
        exec_request.constraints.clone(),
        current_url.clone(),
        PlanSource::ApiChat,
        plan_origin,
        context_snapshot.clone(),
    ) {
        Ok(record) => {
            let store = TaskPlanStore::new(state.default_storage_root());
            match store.save_plan(&record).await {
                Ok(path) => {
                    info!(task_id = %record.task_id, path = %path.display(), "Persisted task plan")
                }
                Err(err) => warn!(?err, "failed to persist task plan"),
            }
        }
        Err(err) => warn!(?err, "failed to serialize task plan for persistence"),
    }

    let mut stdout_lines = vec![format!(
        "Generated plan with {} step(s)",
        current_session.plan.steps.len()
    )];
    let mut stderr_lines: Vec<String> = Vec::new();
    let mut success = true;
    let mut last_execution_report: Option<FlowExecutionReport> = None;
    let mut execution_reports: Vec<FlowExecutionReport> = Vec::new();
    let mut artifacts_value: Option<Value> = None;

    if execute.unwrap_or(false) {
        let task_status_registry = app_context.task_status_registry();
        let handle = task_status_registry.register(
            exec_request.task_id.clone(),
            current_session.plan.title.clone(),
            current_session.plan.steps.len(),
        );
        handle.set_plan_overlays(build_plan_overlays(&current_session.plan));
        if context_snapshot.is_some() {
            handle.set_context_snapshot(context_snapshot.clone());
        }
        let max_replans = max_replans.unwrap_or(0) as u32;
        let mut attempt = 0u32;
        let heal_registry = app_context.self_heal_registry();
        let base_exec_options = FlowExecutionOptions {
            max_retries: 1u8.saturating_add(heal_registry.auto_retry_extra_attempts()),
            progress: Some(handle.clone()),
            self_heal: Some(heal_registry),
            plugin_registry: Some(plugin_registry.clone()),
            ..FlowExecutionOptions::default()
        };

        loop {
            match execute_plan(
                app_context.clone(),
                &exec_request,
                &current_session.plan,
                base_exec_options.clone(),
            )
            .await
            {
                Ok(report) => {
                    execution_reports.push(report.clone());
                    let overlays = build_execution_overlays(&report.steps);
                    handle.push_execution_overlays(overlays);
                    stdout_lines.push(format!(
                        "Execution attempt {} {}",
                        attempt + 1,
                        if report.success {
                            "succeeded"
                        } else {
                            "failed"
                        }
                    ));
                    last_execution_report = Some(report.clone());
                    if report.success {
                        success = true;
                        break;
                    } else {
                        success = false;
                    }

                    if attempt >= max_replans {
                        stderr_lines.push("Plan execution failed".to_string());
                        break;
                    }

                    let observation_summary =
                        latest_observation_summary(&task_status_registry, &exec_request.task_id);
                    let obstruction_kind =
                        latest_obstruction_kind(&task_status_registry, &exec_request.task_id);
                    let agent_history_section =
                        agent_history_prompt(&task_status_registry, &exec_request.task_id, 6);
                    let Some((updated_request, mut failure_summary)) = augment_request_for_replan(
                        &exec_request,
                        &report,
                        attempt,
                        observation_summary.as_deref(),
                        obstruction_kind.as_deref(),
                        agent_history_section.as_deref(),
                    ) else {
                        stderr_lines.push("Plan execution failed".to_string());
                        break;
                    };
                    if let Some(kind) = obstruction_kind.as_deref() {
                        failure_summary.push_str(&format!(" Blocker classification: {kind}."));
                    }

                    stdout_lines.push(format!(
                        "Replanning after attempt {}: {}",
                        attempt + 1,
                        failure_summary
                    ));

                    exec_request = updated_request;
                    apply_blocker_hints(&mut exec_request, obstruction_kind.as_deref());
                    match runner
                        .replan(
                            exec_request.clone(),
                            &current_session.plan,
                            &failure_summary,
                        )
                        .await
                    {
                        Ok(next_session) => {
                            current_session = next_session;
                            plan_history.push(plan_payload(&current_session));
                            handle.update_plan(
                                current_session.plan.title.clone(),
                                current_session.plan.steps.len(),
                            );
                            handle.set_plan_overlays(build_plan_overlays(&current_session.plan));
                            stdout_lines.push(format!(
                                "Generated revised plan with {} step(s)",
                                current_session.plan.steps.len()
                            ));
                        }
                        Err(err) => {
                            success = false;
                            error!(?err, "replanning failed");
                            stderr_lines.push(format!("replan error: {err}"));
                            break;
                        }
                    }

                    attempt += 1;
                }
                Err(err) => {
                    success = false;
                    error!(?err, "plan execution failed");
                    stderr_lines.push(format!("execution error: {err}"));
                    break;
                }
            }
        }

        if !execution_reports.is_empty() {
            let state_events = app_context.state_center_snapshot();
            let telemetry_payload = handle
                .snapshot()
                .as_ref()
                .and_then(|snapshot| build_telemetry_payload(snapshot));
            match persist_execution_outputs(
                &state.config.output_dir,
                &exec_request.task_id,
                &plan_history,
                &execution_reports,
                Some(state_events),
                telemetry_payload,
            )
            .await
            {
                Ok(values) => {
                    if !values.is_empty() {
                        artifacts_value = Some(Value::Array(values));
                    }
                }
                Err(err) => {
                    warn!(?err, "failed to persist chat execution artifacts");
                }
            }
        }
    }

    let plan_value = match serde_json::to_value(&current_session.plan) {
        Ok(val) => val,
        Err(err) => {
            error!(?err, "failed to serialize agent plan");
            let body = ChatResponse {
                success: false,
                plan: None,
                flow: None,
                artifacts: None,
                context: context_snapshot.clone(),
                stdout: stdout_lines.join("\n"),
                stderr: format!("Failed to serialize plan: {err}"),
            };
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(body),
            )
                .into_response();
        }
    };

    let flow_definition_value = match serde_json::to_value(&current_session.flow.flow) {
        Ok(val) => val,
        Err(err) => {
            error!(?err, "failed to serialize flow definition");
            let body = ChatResponse {
                success: false,
                plan: None,
                flow: None,
                artifacts: None,
                context: context_snapshot.clone(),
                stdout: stdout_lines.join("\n"),
                stderr: format!("Failed to serialize flow definition: {err}"),
            };
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(body),
            )
                .into_response();
        }
    };

    let mut flow_value = json!({
        "definition": flow_definition_value,
        "metadata": {
            "step_count": current_session.flow.step_count,
            "validation_count": current_session.flow.validation_count,
        }
    });

    if let Some(report) = last_execution_report.as_ref() {
        flow_value
            .as_object_mut()
            .expect("flow payload is object")
            .insert(
                "execution".to_string(),
                flow_execution_report_payload(report),
            );
    }

    let plan_payload = json!({
        "plan": plan_value,
        "explanations": current_session.explanations,
        "overlays": build_plan_overlays(&current_session.plan),
    });

    if artifacts_value.is_none() {
        if let Some(report) = last_execution_report.as_ref() {
            let evidence = execution_artifacts_from_report(report);
            if !evidence.is_empty() {
                artifacts_value = Some(Value::Array(evidence));
            }
        }
    }

    let response = ChatResponse {
        success,
        plan: Some(plan_payload),
        flow: Some(flow_value),
        artifacts: artifacts_value,
        context: context_snapshot,
        stdout: stdout_lines.join("\n"),
        stderr: stderr_lines.join("\n"),
    };

    let status = if success {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, axum::Json(response)).into_response()
}
