use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get},
    Json, Router,
};
use memory_center::{
    normalize_metadata, normalize_note, normalize_tags, MemoryRecord, MemoryStatsSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::task;
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

    let stored = task::spawn_blocking(move || center.store(record))
        .await
        .expect("memory store task panicked");
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
    let tags_patch = payload.tags.clone().map(normalize_tags);
    let note_field_provided = payload.note.is_some();
    let note_value = normalize_note(payload.note.clone());
    let metadata_field_provided = payload.metadata.is_some();
    let metadata_patch = normalize_metadata(payload.metadata.clone());
    let record_id_clone = record_id.clone();

    let updated = task::spawn_blocking(move || {
        center.update_record(&record_id_clone, |record| {
            if let Some(tags) = tags_patch.as_ref() {
                record.tags = tags.clone();
            }
            if note_field_provided {
                record.note = note_value.clone();
            }
            if metadata_field_provided {
                record.metadata = metadata_patch.clone();
            }
        })
    })
    .await
    .expect("memory update task panicked");

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
    let removal = task::spawn_blocking(move || center.remove_by_id(&record_id))
        .await
        .expect("memory delete task panicked");
    match removal {
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
