use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use memory_center::{MemoryRecord, MemoryStatsSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::instrument;

use crate::server::ServeState;

pub(crate) fn router() -> Router<ServeState> {
    Router::new()
        .route(
            "/api/memory",
            get(memory_list_handler).post(memory_create_handler),
        )
        .route(
            "/api/memory/:record_id",
            delete(memory_delete_handler).put(memory_update_handler),
        )
        .route("/api/memory/stats", get(memory_stats_handler))
}

#[derive(Deserialize)]
struct MemoryListQuery {
    namespace: Option<String>,
    tag: Option<String>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct MemoryListResponse {
    success: bool,
    records: Vec<MemoryRecord>,
}

#[instrument(name = "soul.memory.list", skip(state, query))]
async fn memory_list_handler(
    State(state): State<ServeState>,
    Query(query): Query<MemoryListQuery>,
) -> Json<MemoryListResponse> {
    let context = state.app_context().await;
    let records = context.memory_center().list(
        query.namespace.as_deref(),
        query.tag.as_deref(),
        query.limit,
    );
    Json(MemoryListResponse {
        success: true,
        records,
    })
}

#[derive(Deserialize)]
struct MemoryCreateRequest {
    namespace: String,
    key: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    metadata: Option<Value>,
}

#[derive(Serialize)]
struct MemoryRecordResponse {
    success: bool,
    record: MemoryRecord,
}

#[instrument(name = "soul.memory.create", skip(state, payload))]
async fn memory_create_handler(
    State(state): State<ServeState>,
    Json(payload): Json<MemoryCreateRequest>,
) -> impl IntoResponse {
    if payload.namespace.trim().is_empty() || payload.key.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "error": "namespace and key must not be empty",
            })),
        )
            .into_response();
    }

    let context = state.app_context().await;
    let center = context.memory_center();
    let mut record = MemoryRecord::new(payload.namespace.trim(), payload.key.trim());
    record.tags = normalize_tags(payload.tags);
    record.note = normalize_note(payload.note);
    record.metadata = normalize_metadata(payload.metadata);

    let stored = center.store(record);
    (
        StatusCode::CREATED,
        Json(MemoryRecordResponse {
            success: true,
            record: stored,
        }),
    )
        .into_response()
}

#[derive(Deserialize)]
struct MemoryUpdateRequest {
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    metadata: Option<Value>,
}

#[instrument(name = "soul.memory.update", skip(state, payload))]
async fn memory_update_handler(
    State(state): State<ServeState>,
    Path(record_id): Path<String>,
    Json(payload): Json<MemoryUpdateRequest>,
) -> impl IntoResponse {
    let context = state.app_context().await;
    let center = context.memory_center();
    let updated = center.update_record(&record_id, |record| {
        if let Some(tags) = payload.tags.as_ref() {
            record.tags = normalize_tags(tags.clone());
        }
        if payload.note.is_some() {
            record.note = normalize_note(payload.note.clone());
        }
        if payload.metadata.is_some() {
            record.metadata = normalize_metadata(payload.metadata.clone());
        }
    });

    match updated {
        Some(record) => Json(MemoryRecordResponse {
            success: true,
            record,
        })
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": "memory record not found" })),
        )
            .into_response(),
    }
}

#[instrument(name = "soul.memory.delete", skip(state))]
async fn memory_delete_handler(
    State(state): State<ServeState>,
    Path(record_id): Path<String>,
) -> impl IntoResponse {
    let context = state.app_context().await;
    let center = context.memory_center();
    match center.remove_by_id(&record_id) {
        Some(_) => Json(json!({ "success": true })).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": "memory record not found" })),
        )
            .into_response(),
    }
}

#[derive(Serialize)]
struct MemoryStatsResponse {
    success: bool,
    stats: MemoryStatsSnapshot,
}

#[instrument(name = "soul.memory.stats", skip(state))]
async fn memory_stats_handler(State(state): State<ServeState>) -> Json<MemoryStatsResponse> {
    let context = state.app_context().await;
    let stats = context.memory_center().stats_snapshot();
    Json(MemoryStatsResponse {
        success: true,
        stats,
    })
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    tags.into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn normalize_note(note: Option<String>) -> Option<String> {
    note.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_metadata(metadata: Option<Value>) -> Option<Value> {
    match metadata {
        Some(Value::Null) => None,
        other => other,
    }
}
