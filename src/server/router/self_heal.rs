use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::self_heal::{SelfHealRegistryStats, SelfHealStrategy};
use crate::server::ServeState;
use tracing::instrument;

pub(crate) fn router() -> Router<ServeState> {
    Router::new()
        .route("/api/self-heal/strategies", get(self_heal_list_handler))
        .route(
            "/api/self-heal/strategies/:strategy_id",
            post(self_heal_update_handler),
        )
        .route(
            "/api/self-heal/strategies/:strategy_id/inject",
            post(self_heal_inject_handler),
        )
}

#[derive(Serialize)]
struct SelfHealListResponse {
    success: bool,
    strategies: Vec<SelfHealStrategy>,
    stats: SelfHealRegistryStats,
}

#[instrument(name = "soul.self_heal.list", skip(state))]
async fn self_heal_list_handler(State(state): State<ServeState>) -> Json<SelfHealListResponse> {
    let context = state.app_context().await;
    let registry = context.self_heal_registry();
    Json(SelfHealListResponse {
        success: true,
        strategies: registry.strategies(),
        stats: registry.stats(),
    })
}

#[derive(Deserialize)]
struct SelfHealUpdateRequest {
    enabled: bool,
}

#[derive(Deserialize)]
struct SelfHealInjectRequest {
    #[serde(default)]
    note: Option<String>,
}

#[instrument(name = "soul.self_heal.update", skip(state, payload))]
async fn self_heal_update_handler(
    State(state): State<ServeState>,
    Path(strategy_id): Path<String>,
    Json(payload): Json<SelfHealUpdateRequest>,
) -> impl IntoResponse {
    let context = state.app_context().await;
    let registry = context.self_heal_registry();
    if let Err(err) = registry.set_enabled(&strategy_id, payload.enabled) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": err.to_string() })),
        )
            .into_response();
    }
    Json(json!({ "success": true })).into_response()
}

#[instrument(name = "soul.self_heal.inject", skip(state, payload))]
async fn self_heal_inject_handler(
    State(state): State<ServeState>,
    Path(strategy_id): Path<String>,
    Json(payload): Json<SelfHealInjectRequest>,
) -> impl IntoResponse {
    let context = state.app_context().await;
    let registry = context.self_heal_registry();
    if let Err(err) = registry.inject_event(&strategy_id, payload.note.clone()) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": err.to_string() })),
        )
            .into_response();
    }
    Json(json!({ "success": true })).into_response()
}
