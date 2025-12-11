use std::convert::Infallible;
use std::io::ErrorKind;
use std::time::Duration;

use async_stream::stream;
use axum::response::sse::{Event, KeepAlive};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Sse},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs;
use tokio::sync::broadcast;
use tracing::instrument;

use crate::server::ServeState;
use crate::task_status::{TaskLogEntry, TaskStatusSnapshot, TaskStreamEnvelope};
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
            "/api/tasks/:task_id/executions",
            get(task_executions_handler),
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
        .config
        .output_dir
        .join("tasks")
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
