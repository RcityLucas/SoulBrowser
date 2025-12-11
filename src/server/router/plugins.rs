use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::warn;

use crate::plugin_registry::{
    HelperConditions, HelperStep, HelperTool, PluginRecord, PluginRegistryStats, RegistryError,
    RegistryHelper,
};
use crate::server::ServeState;

pub(crate) fn router() -> Router<ServeState> {
    Router::new()
        .route("/api/plugins/registry", get(plugin_registry_list_handler))
        .route(
            "/api/plugins/registry/:plugin_id",
            post(plugin_registry_update_handler),
        )
        .route(
            "/api/plugins/registry/:plugin_id/helpers",
            get(plugin_registry_helpers_handler).post(plugin_helper_create_handler),
        )
        .route(
            "/api/plugins/registry/:plugin_id/helpers/:helper_id",
            put(plugin_helper_update_handler).delete(plugin_helper_delete_handler),
        )
        .route(
            "/api/plugins/registry/:plugin_id/helpers/scaffold",
            get(plugin_helper_scaffold_handler),
        )
}

#[derive(Serialize)]
struct PluginRegistryResponse {
    success: bool,
    plugins: Vec<PluginRecord>,
    stats: PluginRegistryStats,
}

async fn plugin_registry_list_handler(
    State(state): State<ServeState>,
) -> Json<PluginRegistryResponse> {
    let context = state.app_context().await;
    let registry = context.plugin_registry();
    Json(PluginRegistryResponse {
        success: true,
        plugins: registry.entries(),
        stats: registry.stats(),
    })
}

#[derive(Deserialize)]
struct PluginUpdateRequest {
    status: Option<String>,
    owner: Option<String>,
    description: Option<String>,
    scopes: Option<Vec<String>>,
}

async fn plugin_registry_update_handler(
    State(state): State<ServeState>,
    Path(plugin_id): Path<String>,
    Json(payload): Json<PluginUpdateRequest>,
) -> impl IntoResponse {
    if payload.status.is_none()
        && payload.owner.is_none()
        && payload.description.is_none()
        && payload.scopes.is_none()
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "error": "no fields provided" })),
        )
            .into_response();
    }

    let context = state.app_context().await;
    let registry = context.plugin_registry();
    let mut last_record: Option<PluginRecord> = None;

    if let Some(status) = payload.status.as_ref() {
        match registry.update_status(&plugin_id, status.to_string()) {
            Some(record) => last_record = Some(record),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "success": false, "error": "plugin not found" })),
                )
                    .into_response();
            }
        }
    }

    if payload.owner.is_some() || payload.description.is_some() || payload.scopes.is_some() {
        match registry.update_plugin(&plugin_id, |record| {
            if let Some(owner) = payload.owner.as_ref() {
                record.owner = Some(owner.trim().to_string()).filter(|s| !s.is_empty());
            }
            if let Some(desc) = payload.description.as_ref() {
                record.description = Some(desc.trim().to_string()).filter(|s| !s.is_empty());
            }
            if let Some(scopes) = payload.scopes.as_ref() {
                let cleaned: Vec<String> = scopes
                    .iter()
                    .map(|scope| scope.trim().to_string())
                    .filter(|scope| !scope.is_empty())
                    .collect();
                record.scopes = (!cleaned.is_empty()).then(|| cleaned);
            }
        }) {
            Ok(record) => last_record = Some(record),
            Err(err) => {
                return registry_error(err);
            }
        }
    }

    if let Err(err) = registry.save() {
        warn!(?err, "failed to persist plugin registry after update");
    }

    let Some(record) = last_record else {
        return (
            StatusCode::OK,
            Json(json!({ "success": true, "updated": false })),
        )
            .into_response();
    };

    Json(json!({ "success": true, "plugin": record })).into_response()
}

async fn plugin_registry_helpers_handler(
    State(state): State<ServeState>,
    Path(plugin_id): Path<String>,
) -> impl IntoResponse {
    let context = state.app_context().await;
    let registry = context.plugin_registry();
    match registry.plugin_helpers(&plugin_id) {
        Some(helpers) => Json(json!({ "success": true, "helpers": helpers })).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": "plugin not found" })),
        )
            .into_response(),
    }
}

async fn plugin_helper_create_handler(
    State(state): State<ServeState>,
    Path(plugin_id): Path<String>,
    Json(mut payload): Json<RegistryHelper>,
) -> impl IntoResponse {
    payload.plugin_id = None;
    let context = state.app_context().await;
    let registry = context.plugin_registry();
    match registry.add_helper(&plugin_id, payload) {
        Ok(helper) => {
            if let Err(err) = registry.save() {
                warn!(
                    ?err,
                    "failed to persist plugin registry after helper create"
                );
            }
            Json(json!({ "success": true, "helper": helper })).into_response()
        }
        Err(err) => registry_error(err),
    }
}

async fn plugin_helper_update_handler(
    State(state): State<ServeState>,
    Path((plugin_id, helper_id)): Path<(String, String)>,
    Json(mut payload): Json<RegistryHelper>,
) -> impl IntoResponse {
    payload.plugin_id = None;
    let context = state.app_context().await;
    let registry = context.plugin_registry();
    match registry.update_helper(&plugin_id, &helper_id, payload) {
        Ok(helper) => {
            if let Err(err) = registry.save() {
                warn!(
                    ?err,
                    "failed to persist plugin registry after helper update"
                );
            }
            Json(json!({ "success": true, "helper": helper })).into_response()
        }
        Err(err) => registry_error(err),
    }
}

async fn plugin_helper_delete_handler(
    State(state): State<ServeState>,
    Path((plugin_id, helper_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let context = state.app_context().await;
    let registry = context.plugin_registry();
    match registry.delete_helper(&plugin_id, &helper_id) {
        Ok(()) => {
            if let Err(err) = registry.save() {
                warn!(
                    ?err,
                    "failed to persist plugin registry after helper delete"
                );
            }
            Json(json!({ "success": true })).into_response()
        }
        Err(err) => registry_error(err),
    }
}

async fn plugin_helper_scaffold_handler() -> Json<RegistryHelper> {
    Json(RegistryHelper {
        id: "helper-id".into(),
        pattern: "example\\.com".into(),
        description: Some("Describe what this helper does".into()),
        blockers: Vec::new(),
        auto_insert: true,
        prompt: Some("Narrate why the helper should run".into()),
        step: None,
        steps: vec![HelperStep {
            title: "Click consent".into(),
            detail: Some("Close banner".into()),
            wait: Some("dom_ready".into()),
            timeout_ms: Some(5000),
            tool: HelperTool::ClickCss {
                selector: "button.accept".into(),
            },
        }],
        conditions: HelperConditions::default(),
        plugin_id: None,
    })
}

fn registry_error(err: RegistryError) -> Response {
    let (status, message) = match err {
        RegistryError::PluginNotFound(_) => (StatusCode::NOT_FOUND, err.to_string()),
        RegistryError::HelperNotFound(_) => (StatusCode::NOT_FOUND, err.to_string()),
        RegistryError::HelperExists(_) => (StatusCode::CONFLICT, err.to_string()),
        RegistryError::HelperMissingSteps(_) => (StatusCode::BAD_REQUEST, err.to_string()),
    };
    (status, Json(json!({ "success": false, "error": message }))).into_response()
}
