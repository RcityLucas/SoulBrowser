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
    CreateSessionRequest, SessionLiveEvent, SessionRecord, SessionShareContext, SessionSnapshot,
};
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

fn event_from_live(event: SessionLiveEvent) -> Option<Event> {
    let event_name = match &event {
        SessionLiveEvent::Snapshot { .. } => "snapshot",
        SessionLiveEvent::Status { .. } => "status",
        SessionLiveEvent::Frame { .. } => "frame",
        SessionLiveEvent::Overlay { .. } => "overlay",
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
