use std::time::Duration;

use async_stream::stream;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive};
use axum::response::{IntoResponse, Sse};
use axum::routing::{get, post};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast;
use tracing::warn;

use crate::server::ServeState;
use crate::sessions::{
    CreateSessionRequest, RouteSummary, SessionLiveEvent, SessionRecord, SessionShareContext,
    SessionSnapshot,
};
use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId};
use soulbrowser_registry::Registry;

pub(crate) fn router() -> axum::Router<ServeState> {
    axum::Router::new()
        .route(
            "/api/sessions",
            get(list_sessions_handler).post(create_session_handler),
        )
        .route("/api/sessions/:session_id", get(session_detail_handler))
        .route(
            "/api/sessions/:session_id/share",
            post(issue_share_handler).delete(revoke_share_handler),
        )
        .route("/api/sessions/:session_id/live", get(session_live_handler))
        .route(
            "/api/sessions/:session_id/pointer",
            post(session_pointer_handler),
        )
}

#[derive(Serialize)]
struct SessionListResponse {
    success: bool,
    sessions: Vec<SessionRecord>,
}

async fn list_sessions_handler(State(state): State<ServeState>) -> Json<SessionListResponse> {
    let service = state.session_service().await;
    let mut sessions = service.list();
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Json(SessionListResponse {
        success: true,
        sessions,
    })
}

#[derive(Serialize)]
struct SessionCreateResponse {
    success: bool,
    session: SessionRecord,
}

async fn create_session_handler(
    State(state): State<ServeState>,
    Json(payload): Json<CreateSessionRequest>,
) -> Result<Json<SessionCreateResponse>, (StatusCode, Json<serde_json::Value>)> {
    let context = state.session_service().await;
    let app_ctx = state.app_context().await;
    let registry = app_ctx.registry();
    let profile_hint = payload
        .profile_label
        .clone()
        .or_else(|| payload.profile_id.clone())
        .unwrap_or_else(|| "session".to_string());
    match registry.session_create(&profile_hint).await {
        Ok(session_id) => match context
            .create_session_with_id(session_id.0.clone(), payload)
            .await
        {
            Ok(record) => Ok(Json(SessionCreateResponse {
                success: true,
                session: record,
            })),
            Err(err) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "error": err.to_string(),
                })),
            )),
        },
        Err(err) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "error": err.to_string(),
            })),
        )),
    }
}

#[derive(Serialize)]
struct SessionDetailResponse {
    success: bool,
    snapshot: SessionSnapshot,
}

async fn session_detail_handler(
    State(state): State<ServeState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionDetailResponse>, (StatusCode, Json<serde_json::Value>)> {
    let service = state.session_service().await;
    let Some(snapshot) = service.snapshot(&session_id) else {
        return Err(not_found("session not found"));
    };
    Ok(Json(SessionDetailResponse {
        success: true,
        snapshot,
    }))
}

#[derive(Serialize)]
struct SessionShareResponse {
    success: bool,
    share: SessionShareContext,
}

async fn issue_share_handler(
    State(state): State<ServeState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionShareResponse>, (StatusCode, Json<serde_json::Value>)> {
    let service = state.session_service().await;
    match service.issue_share_link(&session_id).await {
        Ok(ctx) => Ok(Json(SessionShareResponse {
            success: true,
            share: ctx,
        })),
        Err(_) => Err(not_found("session not found")),
    }
}

async fn revoke_share_handler(
    State(state): State<ServeState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionShareResponse>, (StatusCode, Json<serde_json::Value>)> {
    let service = state.session_service().await;
    match service.revoke_share_link(&session_id).await {
        Ok(ctx) => Ok(Json(SessionShareResponse {
            success: true,
            share: ctx,
        })),
        Err(_) => Err(not_found("session not found")),
    }
}

#[derive(Deserialize)]
struct SessionLiveQuery {
    share: Option<String>,
}

async fn session_live_handler(
    State(state): State<ServeState>,
    Path(session_id): Path<String>,
    Query(query): Query<SessionLiveQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let service = state.session_service().await;
    let Some(snapshot) = service.snapshot(&session_id) else {
        return Err(not_found("session not found"));
    };
    if let Some(expected) = snapshot.session.share_token.as_deref() {
        match query.share.as_deref() {
            Some(token) if token == expected => {}
            Some(_) => {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"success": false, "error": "invalid share token"})),
                ))
            }
            None => {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"success": false, "error": "share token required"})),
                ))
            }
        }
    }
    let Some(mut receiver) = service.subscribe(&session_id) else {
        return Err(not_found("live stream unavailable"));
    };
    let initial_event = SessionLiveEvent::Snapshot {
        snapshot: snapshot.clone(),
    };
    let stream = stream! {
        if let Some(event) = event_from_live(initial_event) {
            yield Ok::<Event, std::convert::Infallible>(event);
        }
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if let Some(serialized) = event_from_live(event) {
                        yield Ok(serialized);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(err) => {
                    warn!(?err, "session live stream closed");
                    break;
                }
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

#[derive(Deserialize)]
struct SessionPointerRequest {
    action: String,
    x: f64,
    y: f64,
    #[serde(default)]
    button: Option<String>,
    #[serde(default)]
    delta_x: Option<f64>,
    #[serde(default)]
    delta_y: Option<f64>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    route: Option<RouteSummary>,
}

#[derive(Serialize)]
struct SessionPointerResponse {
    success: bool,
}

async fn session_pointer_handler(
    State(state): State<ServeState>,
    Path(session_id): Path<String>,
    Json(payload): Json<SessionPointerRequest>,
) -> Result<Json<SessionPointerResponse>, (StatusCode, Json<serde_json::Value>)> {
    let service = state.session_service().await;
    let Some(snapshot) = service.snapshot(&session_id) else {
        return Err(not_found("session not found"));
    };
    let route_summary = payload
        .route
        .clone()
        .or_else(|| {
            snapshot
                .last_frame
                .as_ref()
                .and_then(|frame| frame.route.clone())
        })
        .ok_or_else(|| bad_request("route information is required"))?;
    if route_summary.session != session_id {
        return Err(bad_request("route session mismatch"));
    }
    let page_id = route_summary
        .page
        .clone()
        .ok_or_else(|| bad_request("route page missing"))?;
    let frame_id = route_summary
        .frame
        .clone()
        .ok_or_else(|| bad_request("route frame missing"))?;
    let exec_route = ExecRoute::new(
        SessionId(session_id.clone()),
        PageId(page_id.clone()),
        FrameId(frame_id.clone()),
    );
    let subject_id = frame_id.clone();

    let mut map = serde_json::Map::new();
    map.insert(
        "action".into(),
        serde_json::Value::String(payload.action.clone()),
    );
    map.insert("x".into(), serde_json::Value::from(payload.x));
    map.insert("y".into(), serde_json::Value::from(payload.y));
    if let Some(button) = payload.button {
        map.insert("button".into(), serde_json::Value::String(button));
    }
    if let Some(delta_x) = payload.delta_x {
        map.insert("delta_x".into(), serde_json::Value::from(delta_x));
    }
    if let Some(delta_y) = payload.delta_y {
        map.insert("delta_y".into(), serde_json::Value::from(delta_y));
    }
    if let Some(text) = payload.text {
        map.insert("text".into(), serde_json::Value::String(text));
    }

    let app_context = state.app_context().await;
    let tool_manager = app_context.tool_manager();
    tool_manager
        .execute_with_route(
            "manual.pointer",
            &subject_id,
            serde_json::Value::Object(map),
            Some(exec_route),
            Some(3_000),
        )
        .await
        .map_err(|err| internal_error(&format!("pointer dispatch failed: {}", err)))?;

    Ok(Json(SessionPointerResponse { success: true }))
}

fn event_from_live(event: SessionLiveEvent) -> Option<Event> {
    let event_name = match &event {
        SessionLiveEvent::Snapshot { .. } => "snapshot",
        SessionLiveEvent::Status { .. } => "status",
        SessionLiveEvent::Frame { .. } => "frame",
        SessionLiveEvent::Overlay { .. } => "overlay",
        SessionLiveEvent::MessageState { .. } => "message_state",
    };
    match serde_json::to_string(&event) {
        Ok(payload) => Some(Event::default().event(event_name).data(payload)),
        Err(err) => {
            warn!(?err, "failed to serialize live session event");
            None
        }
    }
}

fn not_found(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "success": false,
            "error": message,
        })),
    )
}

fn bad_request(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "success": false,
            "error": message,
        })),
    )
}

fn internal_error(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "success": false,
            "error": message,
        })),
    )
}
