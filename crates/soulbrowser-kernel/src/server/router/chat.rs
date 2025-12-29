use std::net::SocketAddr;

use agent_core::convert::{plan_to_flow, PlanToFlowOptions};
use agent_core::plan::{AgentPlan, AgentPlanStep, AgentToolKind};
use agent_core::AgentContext;
use axum::{extract::ConnectInfo, response::IntoResponse, routing::post, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{error, info, instrument, warn};

use crate::agent::{execute_plan, FlowExecutionOptions, FlowExecutionReport};
use crate::chat_support::{
    apply_perception_metadata, build_chat_runner, build_telemetry_payload,
    capture_chat_context_snapshot, flow_execution_report_payload, llm_cache_for_request,
    perception_snapshot_value, persist_execution_outputs, propagate_browser_state_metadata,
    sync_agent_execution_history, ChatRunnerBuild, LlmProviderConfig, LlmProviderSelection,
    PlannerSelection,
};
use crate::intent::enrich_request_with_intent;
use crate::judge;
use crate::plan_payload;
use crate::server::{rate_limit::RateLimitKind, ServeState};
use crate::sessions::CreateSessionRequest;
use crate::task_store::{PersistedPlanRecord, PlanOriginMetadata, PlanSource, TaskPlanStore};
use crate::visualization::{
    build_execution_overlays, build_plan_overlays, execution_artifacts_from_report,
};
use soulbrowser_core_types::SessionId;
use soulbrowser_registry::Registry;
use soulbrowser_scheduler::model::Priority;

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
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    profile_label: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    stdout: String,
    stderr: String,
}

#[instrument(
    name = "soul.chat.request",
    skip(state, req),
    fields(client_ip = %client_addr)
)]
async fn serve_chat_handler(
    axum::extract::State(state): axum::extract::State<ServeState>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    axum::Json(req): axum::Json<ChatRequest>,
) -> impl axum::response::IntoResponse {
    let mut response_session_id: Option<String> = None;
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
            session_id: response_session_id.clone(),
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
        max_replans: _max_replans,
        capture_context,
        context_timeout_secs,
        context_screenshot,
        session_id,
        profile_id,
        profile_label,
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
                session_id: response_session_id.clone(),
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
                session_id: response_session_id.clone(),
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
            session_id: response_session_id.clone(),
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
            session_id: response_session_id.clone(),
            stdout: String::new(),
            stderr: "Prompt exceeds 2000 characters".to_string(),
        };
        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
    }

    info!("chat request received");
    let requested_execution = execute.unwrap_or(false);
    let lightweight_reply = lightweight_chat_reply(trimmed_prompt);
    let should_execute = requested_execution && lightweight_reply.is_none();
    if let Some(reply) = lightweight_reply {
        if !should_execute {
            let body = ChatResponse {
                success: true,
                plan: None,
                flow: None,
                artifacts: None,
                context: None,
                session_id: None,
                stdout: reply,
                stderr: String::new(),
            };
            return (axum::http::StatusCode::OK, axum::Json(body)).into_response();
        }
    }

    let app_context = match state.app_context().await {
        ctx => ctx,
    };
    let session_service = app_context.session_service();
    let registry = app_context.registry();
    let truncated_description: String = trimmed_prompt.chars().take(120).collect();
    let profile_hint = profile_label
        .clone()
        .or_else(|| profile_id.clone())
        .unwrap_or_else(|| "chat-session".to_string());

    let active_session_id = if let Some(existing_id) = session_id {
        let session_exists = registry
            .session_list()
            .await
            .into_iter()
            .any(|ctx| ctx.id.0 == existing_id);
        if !session_exists {
            let body = ChatResponse {
                success: false,
                plan: None,
                flow: None,
                artifacts: None,
                context: None,
                session_id: Some(existing_id.clone()),
                stdout: String::new(),
                stderr: "Session not found".to_string(),
            };
            return (axum::http::StatusCode::NOT_FOUND, axum::Json(body)).into_response();
        }
        if session_service.get(&existing_id).is_none() {
            if let Err(err) = session_service
                .create_session_with_id(
                    existing_id.clone(),
                    CreateSessionRequest {
                        profile_id: profile_id.clone(),
                        profile_label: profile_label.clone(),
                        description: if truncated_description.is_empty() {
                            None
                        } else {
                            Some(truncated_description.clone())
                        },
                        shared: None,
                    },
                )
                .await
            {
                warn!(?err, session = %existing_id, "failed to persist session metadata");
            }
        }
        existing_id
    } else {
        match registry.session_create(&profile_hint).await {
            Ok(new_id) => {
                let id_str = new_id.0.clone();
                if let Err(err) = session_service
                    .create_session_with_id(
                        id_str.clone(),
                        CreateSessionRequest {
                            profile_id: profile_id.clone(),
                            profile_label: profile_label.clone(),
                            description: if truncated_description.is_empty() {
                                None
                            } else {
                                Some(truncated_description.clone())
                            },
                            shared: None,
                        },
                    )
                    .await
                {
                    error!(?err, "failed to create session for chat request");
                    let body = ChatResponse {
                        success: false,
                        plan: None,
                        flow: None,
                        artifacts: None,
                        context: None,
                        session_id: None,
                        stdout: String::new(),
                        stderr: "Failed to create session".to_string(),
                    };
                    return (
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        axum::Json(body),
                    )
                        .into_response();
                }
                id_str
            }
            Err(err) => {
                error!(?err, "registry failed to create session");
                let body = ChatResponse {
                    success: false,
                    plan: None,
                    flow: None,
                    artifacts: None,
                    context: None,
                    session_id: None,
                    stdout: String::new(),
                    stderr: "Failed to create session".to_string(),
                };
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(body),
                )
                    .into_response();
            }
        }
    };
    response_session_id = Some(active_session_id.clone());
    let plugin_registry = app_context.plugin_registry();

    let mut agent_context = AgentContext::default();
    agent_context.session = Some(SessionId(active_session_id.clone()));
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

    let runner_build = match build_chat_runner(
        planner_choice,
        llm_choice,
        llm_config,
        cache_for_request,
        Some(plugin_registry.clone()),
    ) {
        Ok(build) => build,
        Err(err) => {
            error!(?err, "failed to configure chat runner");
            let body = ChatResponse {
                success: false,
                plan: None,
                flow: None,
                artifacts: None,
                context: None,
                session_id: response_session_id.clone(),
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
    let ChatRunnerBuild {
        runner,
        planner_used: actual_planner,
        provider_used: actual_llm_provider,
        fallback_reason,
    } = runner_build;
    let fallback_warning = fallback_reason.clone();

    let constraint_list = constraints.unwrap_or_default();
    if constraint_list.len() > 10 {
        let body = ChatResponse {
            success: false,
            plan: None,
            flow: None,
            artifacts: None,
            context: None,
            session_id: response_session_id.clone(),
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
            session_id: response_session_id.clone(),
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
    agent_request
        .metadata
        .insert("execute_requested".to_string(), Value::Bool(should_execute));
    agent_request.metadata.insert(
        "session_id".to_string(),
        Value::String(active_session_id.clone()),
    );
    if let Some(note) = fallback_warning.as_ref() {
        agent_request
            .metadata
            .insert("planner_warning".to_string(), Value::String(note.clone()));
    }
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
                session_id: response_session_id.clone(),
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

    let current_session = session;
    let exec_request = agent_request.clone();
    let plan_history = vec![plan_payload(&current_session)];
    session_service.bind_task(&active_session_id, &exec_request.task_id.0);

    let plan_origin = PlanOriginMetadata {
        planner: actual_planner.label().to_string(),
        llm_provider: actual_llm_provider.map(|p| p.label().to_string()),
        llm_model: actual_llm_provider.and_then(|_| llm_model_for_origin.clone()),
    };

    match PersistedPlanRecord::from_session(
        &current_session,
        trimmed_prompt,
        exec_request.constraints.clone(),
        current_url.clone(),
        PlanSource::ApiChat,
        plan_origin,
        context_snapshot.clone(),
        response_session_id.as_deref(),
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
    if let Some(note) = fallback_warning {
        stderr_lines.push(note);
    }
    let mut success = true;
    let mut last_execution_report: Option<FlowExecutionReport> = None;
    let mut execution_reports: Vec<FlowExecutionReport> = Vec::new();
    let mut artifacts_value: Option<Value> = None;

    if should_execute {
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
        handle.mark_running();

        let heal_registry = app_context.self_heal_registry();
        let exec_options = FlowExecutionOptions {
            max_retries: 1u8.saturating_add(heal_registry.auto_retry_extra_attempts()),
            priority: Priority::Standard,
        };

        match execute_plan(
            app_context.clone(),
            &exec_request,
            &current_session.plan,
            exec_options,
        )
        .await
        {
            Ok(report) => {
                execution_reports.push(report.clone());
                let verdict = judge::evaluate_plan(&exec_request, &report);
                handle.set_judge_verdict(verdict.clone());
                if !verdict.passed {
                    warn!(
                        task = %exec_request.task_id.0,
                        reason = verdict.reason.as_deref().unwrap_or("unspecified"),
                        "judge flagged execution outcome"
                    );
                }
                sync_agent_execution_history(&handle, &report);
                let overlays = build_execution_overlays(&report.steps);
                let judge_overlays =
                    crate::agent_judge::build_judge_overlays(&exec_request, &report);
                crate::agent_judge::emit_judge_overlays(&handle, judge_overlays);
                handle.push_execution_overlays(overlays);
                stdout_lines.push(format!(
                    "Execution {}",
                    if report.success {
                        "succeeded"
                    } else {
                        "failed"
                    }
                ));
                last_execution_report = Some(report.clone());
                success = report.success;
            }
            Err(err) => {
                let err_text = err.to_string();
                if let Some(note) = browser_execution_blocked_message(&err_text) {
                    stdout_lines.push(note.to_string());
                    warn!(%note, "skipping execution because browser session is unavailable");
                    handle.mark_success();
                } else {
                    success = false;
                    error!(?err, "plan execution failed");
                    handle.mark_failure(Some(err_text.clone()));
                    stderr_lines.push(format!("execution error: {err_text}"));
                }
            }
        }

        if !execution_reports.is_empty() {
            let state_events = app_context.state_center_snapshot();
            let telemetry_payload = handle
                .snapshot()
                .as_ref()
                .and_then(|snapshot| build_telemetry_payload(snapshot));
            let execution_root = state.execution_output_root();
            match persist_execution_outputs(
                &execution_root,
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
                session_id: response_session_id.clone(),
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
                session_id: response_session_id.clone(),
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
        session_id: response_session_id.clone(),
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

fn browser_execution_blocked_message(error: &str) -> Option<&'static str> {
    let lowered = error.to_ascii_lowercase();
    if lowered.contains("no pages for session") {
        Some("Execution skipped: 当前没有可用的浏览器会话，请先连接 Chrome/Chromium 或附加 DevTools 端口。")
    } else if lowered.contains("cdp adapter") && lowered.contains("stub mode") {
        Some("Execution skipped: CDP 适配器仍处于 stub 模式，尚未连接真实浏览器。")
    } else {
        None
    }
}

fn lightweight_chat_reply(prompt: &str) -> Option<String> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() || trimmed.len() > 80 {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    const ACTION_HINTS: &[&str] = &[
        "打开", "访问", "浏览", "执行", "运行", "search", "navigate", "visit", "download",
    ];
    if ACTION_HINTS.iter().any(|hint| {
        let needle = hint.to_ascii_lowercase();
        trimmed.contains(hint) || lower.contains(&needle)
    }) {
        return None;
    }
    if trimmed.contains('？') || trimmed.contains('?') {
        const QUESTION_HINTS: &[&str] = &["你是谁", "你会什么", "在吗", "你可以", "能做什么"];
        if QUESTION_HINTS.iter().any(|hint| trimmed.contains(hint)) {
            return Some("我在这里，可以帮你规划和执行浏览任务，有需要尽管说~".to_string());
        }
    }
    if trimmed.contains("谢谢") || lower.contains("thanks") {
        return Some("不客气，很高兴能帮到你。".to_string());
    }
    if trimmed.contains("辛苦") {
        return Some("一点也不辛苦，希望继续为你服务。".to_string());
    }
    if trimmed.contains("你好") || lower.contains("hello") || lower.contains("hi") {
        return Some(
            "你好！我是 SoulBrowser 的小助手，告诉我你的目标就可以开始规划啦。".to_string(),
        );
    }
    if trimmed.len() <= 12 {
        return Some("收到～如果需要我动手操作网页，只要描述任务即可。".to_string());
    }
    None
}
