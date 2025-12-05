use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use serde_json::json;
use tracing::error;

use crate::server::ServeState;

pub(crate) fn router() -> Router<ServeState> {
    Router::new()
        .route("/api/admin/context/refresh", post(refresh_context_handler))
        .route("/api/admin/memory/persist", post(persist_memory_handler))
}

async fn refresh_context_handler(State(state): State<ServeState>) -> impl IntoResponse {
    match state.refresh_app_context().await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "tenant": state.tenant_id(),
            })),
        )
            .into_response(),
        Err(err) => {
            error!(?err, "failed to refresh app context");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "error": err.to_string(),
                })),
            )
                .into_response()
        }
    }
}

async fn persist_memory_handler(State(state): State<ServeState>) -> impl IntoResponse {
    let context = state.app_context().await;
    let memory_center = context.memory_center();
    match tokio::task::spawn_blocking(move || memory_center.persist_now()).await {
        Ok(Ok(())) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": "memory center persisted",
            })),
        )
            .into_response(),
        Ok(Err(err)) => {
            error!(?err, "memory center persistence failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "error": err.to_string(),
                })),
            )
                .into_response()
        }
        Err(err) => {
            error!(?err, "memory center persistence task panicked");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "error": "memory persistence join error",
                })),
            )
                .into_response()
        }
    }
}
