use std::convert::Infallible;
use std::io::ErrorKind;
use std::time::Duration;

use async_stream::stream;
use axum::response::sse::{Event, KeepAlive};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response, Sse},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs;
use tokio::sync::broadcast;
use tracing::{instrument, warn};

use cdp_adapter::{AdapterMode, DebuggerEndpoint, PageId as AdapterPageId};
use soulbrowser_core_types::TaskId;
use uuid::Uuid;

use crate::manual_override::{
    ManualOverrideError, ManualOverridePhase, ManualOverrideSnapshot, ManualRouteContext,
    ManualTakeoverRequest,
};
use crate::metrics::record_manual_takeover_triggered;
use crate::server::ServeState;
use crate::task_status::{ExecutionStatus, TaskLogEntry, TaskStatusSnapshot, TaskStreamEnvelope};
use crate::task_store::{PlanSummary, TaskPlanStore};

pub(crate) fn router() -> Router<ServeState> {
    Router::new()
        .route(
            "/api/tasks",
            get(list_tasks_handler).post(create_task_handler),
        )
        .route("/api/tasks/:task_id", get(get_task_handler))
        .route("/api/tasks/:task_id/status", get(task_status_handler))
        .route("/api/tasks/:task_id/logs", get(task_logs_handler))
        .route("/api/tasks/:task_id/events", get(task_events_sse_handler))
        .route("/api/tasks/:task_id/stream", get(task_events_sse_handler))
        .route(
            "/api/tasks/:task_id/message_state",
            get(task_message_state_handler),
        )
        .route(
            "/api/tasks/:task_id/executions",
            get(task_executions_handler),
        )
        .route(
            "/api/tasks/:task_id/manual_takeover",
            post(task_manual_takeover_handler),
        )
        .route(
            "/api/tasks/:task_id/manual_takeover/resume",
            post(task_manual_takeover_resume_handler),
        )
}

#[derive(Serialize)]
struct TaskListResponse {
    success: bool,
    tasks: Vec<PlanSummary>,
}

#[instrument(name = "soul.tasks.list", skip(state))]
async fn list_tasks_handler(State(state): State<ServeState>) -> impl IntoResponse {
    let store = TaskPlanStore::new(state.default_storage_root());
    match store.list_plan_summaries().await {
        Ok(mut summaries) => {
            summaries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            Json(TaskListResponse {
                success: true,
                tasks: summaries,
            })
            .into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "success": false, "error": err.to_string() })),
        )
            .into_response(),
    }
}

async fn create_task_handler() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({ "success": false, "error": "task creation via API is not available" })),
    )
}

#[derive(Serialize)]
struct TaskDetailResponse {
    success: bool,
    task: crate::task_store::PersistedPlanRecord,
}

#[derive(Serialize)]
struct TaskExecutionsResponse {
    success: bool,
    executions: Vec<Value>,
}

#[derive(Serialize)]
struct TaskMessageStateResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_state: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

#[instrument(name = "soul.tasks.detail", skip(state))]
async fn get_task_handler(
    State(state): State<ServeState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let store = TaskPlanStore::new(state.default_storage_root());
    match store.load_plan(&task_id).await {
        Ok(record) => Json(TaskDetailResponse {
            success: true,
            task: record,
        })
        .into_response(),
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": err.to_string() })),
        )
            .into_response(),
    }
}

#[instrument(name = "soul.tasks.executions", skip(state))]
async fn task_executions_handler(
    State(state): State<ServeState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let path = state
        .execution_output_root()
        .join(&task_id)
        .join("executions.json");

    match fs::read(&path).await {
        Ok(bytes) => match serde_json::from_slice::<Vec<Value>>(&bytes) {
            Ok(executions) => Json(TaskExecutionsResponse {
                success: true,
                executions,
            })
            .into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "success": false, "error": err.to_string() })),
            )
                .into_response(),
        },
        Err(err) if err.kind() == ErrorKind::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": "executions not found" })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "success": false, "error": err.to_string() })),
        )
            .into_response(),
    }
}

#[instrument(name = "soul.tasks.message_state", skip(state))]
async fn task_message_state_handler(
    State(state): State<ServeState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let registry = state.task_status_registry().await;
    if let Some(snapshot) = registry.snapshot(&task_id) {
        if let Some(value) = snapshot.message_state.clone() {
            return Json(TaskMessageStateResponse {
                success: true,
                message_state: Some(value),
                source: Some("live".to_string()),
            })
            .into_response();
        }
    }

    let path = state
        .execution_output_root()
        .join(&task_id)
        .join("message_state.json");
    match fs::read(&path).await {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => Json(TaskMessageStateResponse {
                success: true,
                message_state: Some(value),
                source: Some("artifact".to_string()),
            })
            .into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "error": format!("failed to parse message_state.json: {err}"),
                })),
            )
                .into_response(),
        },
        Err(err) => {
            if err.kind() == ErrorKind::NotFound {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({
                        "success": false,
                        "error": "message_state not available",
                    })),
                )
                    .into_response();
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "success": false, "error": err.to_string() })),
            )
                .into_response()
        }
    }
}

#[derive(Deserialize, Default)]
struct ManualTakeoverBody {
    #[serde(default)]
    requested_by: Option<String>,
    #[serde(default)]
    expires_in_secs: Option<u64>,
}

#[derive(Deserialize)]
struct ManualTakeoverResumeBody {
    resume_token: String,
}

#[derive(Serialize)]
struct ManualTakeoverApiResponse {
    success: bool,
    takeover: ManualOverrideView,
}

#[derive(Serialize)]
struct ManualOverrideView {
    status: ManualOverridePhase,
    requested_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    activated_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resumed_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_by: Option<String>,
    route: ManualRouteContext,
    debugger: ManualDebuggerView,
    #[serde(skip_serializing_if = "Option::is_none")]
    resume_token: Option<String>,
}

#[derive(Serialize)]
struct ManualDebuggerView {
    ws_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    inspect_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    launch_url: Option<String>,
}

#[instrument(
    name = "soul.tasks.manual_takeover",
    skip(state, body),
    fields(task_id = %task_id)
)]
async fn task_manual_takeover_handler(
    State(state): State<ServeState>,
    Path(task_id): Path<String>,
    Json(body): Json<ManualTakeoverBody>,
) -> impl IntoResponse {
    let manager = state.manual_session_manager().await;
    if !manager.config().enabled {
        return manual_error_response(StatusCode::FORBIDDEN, "manual takeover disabled");
    }

    let registry = state.task_status_registry().await;
    let Some(handle) = registry.handle(TaskId(task_id.clone())) else {
        return manual_error_response(StatusCode::NOT_FOUND, "task not found");
    };
    let Some(status) = registry.snapshot(&task_id) else {
        return manual_error_response(StatusCode::NOT_FOUND, "task not found");
    };
    if !matches!(
        status.status,
        ExecutionStatus::Running | ExecutionStatus::Pending
    ) {
        return manual_error_response(StatusCode::CONFLICT, "task is not running");
    }

    let Some(route) = registry.latest_route_context(&task_id) else {
        warn!(task = %task_id, "manual takeover requested but no route available");
        return manual_error_response(
            StatusCode::CONFLICT,
            "no recent browser route available for task",
        );
    };

    let Some(page_id) = parse_adapter_page(&route) else {
        warn!(task = %task_id, "manual takeover route missing page identifier");
        return manual_error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "task route missing page identifier",
        );
    };

    let app_ctx = state.app_context().await;
    let tools = app_ctx.tool_manager();
    if matches!(tools.adapter_mode(), Some(AdapterMode::Stub)) {
        warn!(task = %task_id, "manual takeover requested while adapter in stub mode");
        return manual_error_response(
            StatusCode::FAILED_DEPENDENCY,
            "browser not running in real mode",
        );
    }
    let Some(adapter) = tools.cdp_adapter() else {
        return manual_error_response(StatusCode::FAILED_DEPENDENCY, "browser adapter unavailable");
    };

    let Some(debugger) = adapter.debugger_endpoint(page_id).await else {
        warn!(task = %task_id, "manual takeover debugger endpoint unavailable");
        return manual_error_response(
            StatusCode::CONFLICT,
            "debugger endpoint not available for page",
        );
    };

    let expires_override = body
        .expires_in_secs
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs);
    let takeover_request = ManualTakeoverRequest {
        task_id: TaskId(task_id.clone()),
        debugger: debugger.clone(),
        route: route.clone(),
        requested_by: body.requested_by.clone(),
        expires_in: expires_override,
    };

    let response = match manager.request_takeover(takeover_request) {
        Ok(resp) => resp,
        Err(err) => {
            let (status, message) = map_manual_override_error(err);
            return manual_error_response(status, message);
        }
    };

    let snapshot = response.snapshot.clone();
    let view = build_manual_override_view(
        &snapshot,
        debugger,
        route,
        Some(response.resume_token.clone()),
    );
    handle.update_manual_override(snapshot);
    record_manual_takeover_triggered(
        body.requested_by
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("api"),
    );

    Json(ManualTakeoverApiResponse {
        success: true,
        takeover: view,
    })
    .into_response()
}

#[instrument(
    name = "soul.tasks.manual_takeover_resume",
    skip(state, body),
    fields(task_id = %task_id)
)]
async fn task_manual_takeover_resume_handler(
    State(state): State<ServeState>,
    Path(task_id): Path<String>,
    Json(body): Json<ManualTakeoverResumeBody>,
) -> impl IntoResponse {
    let manager = state.manual_session_manager().await;
    if !manager.config().enabled {
        return manual_error_response(StatusCode::FORBIDDEN, "manual takeover disabled");
    }

    let registry = state.task_status_registry().await;
    let Some(handle) = registry.handle(TaskId(task_id.clone())) else {
        return manual_error_response(StatusCode::NOT_FOUND, "task not found");
    };

    let result = match manager.resume(&TaskId(task_id.clone()), &body.resume_token) {
        Ok(snapshot) => snapshot,
        Err(err) => {
            let (status, message) = map_manual_override_error(err);
            return manual_error_response(status, message);
        }
    };

    let debugger = match result.debugger.clone() {
        Some(dbg) => dbg,
        None => {
            warn!(task = %task_id, "manual takeover snapshot missing debugger info");
            return manual_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "missing debugger endpoint",
            );
        }
    };
    let route = match result.route.clone() {
        Some(route) => route,
        None => {
            warn!(task = %task_id, "manual takeover snapshot missing route info");
            return manual_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "missing route context",
            );
        }
    };

    let view = build_manual_override_view(&result, debugger, route, None);
    handle.update_manual_override(result);

    Json(ManualTakeoverApiResponse {
        success: true,
        takeover: view,
    })
    .into_response()
}

fn manual_error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "success": false, "error": message.into() })),
    )
        .into_response()
}

fn parse_adapter_page(route: &ManualRouteContext) -> Option<AdapterPageId> {
    let page = route.page.as_deref()?;
    let uuid = Uuid::parse_str(page).ok()?;
    Some(AdapterPageId(uuid))
}

fn map_manual_override_error(err: ManualOverrideError) -> (StatusCode, String) {
    match err {
        ManualOverrideError::Disabled => (StatusCode::FORBIDDEN, "manual takeover disabled".into()),
        ManualOverrideError::AlreadyActive => (
            StatusCode::CONFLICT,
            "manual takeover already active for task".into(),
        ),
        ManualOverrideError::NotFound => (
            StatusCode::NOT_FOUND,
            "manual takeover not active for task".into(),
        ),
        ManualOverrideError::InvalidToken => {
            (StatusCode::UNAUTHORIZED, "resume token invalid".into())
        }
        ManualOverrideError::Expired => (StatusCode::GONE, "manual takeover expired".into()),
    }
}

fn build_manual_override_view(
    snapshot: &ManualOverrideSnapshot,
    debugger: DebuggerEndpoint,
    route: ManualRouteContext,
    resume_token: Option<String>,
) -> ManualOverrideView {
    let debugger_view = ManualDebuggerView {
        ws_url: debugger.ws_url.clone(),
        inspect_url: debugger.inspect_url.clone(),
        launch_url: debugger.inspect_url.clone(),
    };

    ManualOverrideView {
        status: snapshot.status,
        requested_at: snapshot.requested_at,
        activated_at: snapshot.activated_at,
        resumed_at: snapshot.resumed_at,
        expires_at: snapshot.expires_at,
        requested_by: snapshot.requested_by.clone(),
        route,
        debugger: debugger_view,
        resume_token,
    }
}

#[derive(Serialize)]
struct TaskStatusResponse {
    success: bool,
    status: TaskStatusSnapshot,
}

#[instrument(name = "soul.tasks.status", skip(state))]
async fn task_status_handler(
    State(state): State<ServeState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let registry = state.task_status_registry().await;
    match registry.snapshot(&task_id) {
        Some(snapshot) => Json(TaskStatusResponse {
            success: true,
            status: snapshot,
        })
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": "task not found" })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct TaskLogsQuery {
    cursor: Option<u64>,
    limit: Option<usize>,
    since: Option<String>,
}

#[derive(Serialize)]
struct TaskLogsResponse {
    success: bool,
    logs: Vec<TaskLogEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<u64>,
}

#[instrument(name = "soul.tasks.logs", skip(state, query))]
async fn task_logs_handler(
    State(state): State<ServeState>,
    Path(task_id): Path<String>,
    Query(query): Query<TaskLogsQuery>,
) -> impl IntoResponse {
    let registry = state.task_status_registry().await;
    let since = match parse_since(query.since.as_deref()) {
        Ok(value) => value,
        Err(response) => return response.into_response(),
    };
    let (logs, next_cursor) = match registry.logs_since(&task_id, since, query.cursor, query.limit)
    {
        Some(result) => result,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "success": false, "error": "task not found" })),
            )
                .into_response();
        }
    };

    Json(TaskLogsResponse {
        success: true,
        logs,
        next_cursor,
    })
    .into_response()
}

#[derive(Deserialize)]
struct TaskEventsQuery {
    cursor: Option<u64>,
}

#[instrument(
    name = "soul.tasks.stream",
    skip(state, headers, query),
    fields(task_id = %task_id)
)]
async fn task_events_sse_handler(
    State(state): State<ServeState>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<TaskEventsQuery>,
) -> impl IntoResponse {
    let registry = state.task_status_registry().await;
    if registry.snapshot(&task_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": "task not found" })),
        )
            .into_response();
    }

    let header_cursor = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.parse::<u64>().ok());
    let cursor = query.cursor.or(header_cursor);

    let history = registry
        .stream_history_since(&task_id, cursor)
        .unwrap_or_default();
    let mut receiver = match registry.subscribe(&task_id) {
        Some(rx) => rx,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "success": false, "error": "task stream unavailable" })),
            )
                .into_response();
        }
    };

    let stream = stream! {
        for envelope in history {
            yield Ok::<Event, Infallible>(event_from_envelope(envelope));
        }
        loop {
            match receiver.recv().await {
                Ok(envelope) => yield Ok(event_from_envelope(envelope)),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response()
}

fn event_from_envelope(envelope: TaskStreamEnvelope) -> Event {
    let event_json = serde_json::to_string(&envelope.event).unwrap_or_else(|_| "{}".to_string());
    Event::default()
        .id(envelope.id.to_string())
        .event(envelope.event.kind())
        .data(event_json)
}

fn parse_since(raw: Option<&str>) -> Result<Option<DateTime<Utc>>, (StatusCode, Json<Value>)> {
    if let Some(value) = raw {
        if value.trim().is_empty() {
            return Ok(None);
        }
        match DateTime::parse_from_rfc3339(value) {
            Ok(dt) => Ok(Some(dt.with_timezone(&Utc))),
            Err(err) => Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "error": format!("invalid since timestamp: {}", err),
                })),
            )),
        }
    } else {
        Ok(None)
    }
}
