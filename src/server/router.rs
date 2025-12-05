use axum::{extract::State, http::Method, response::IntoResponse, routing::get, Json, Router};
use serde_json::{json, Value};
use tower_http::cors::{Any, CorsLayer};

use crate::{metrics, CONSOLE_HTML};
use prometheus::{Encoder, TextEncoder};
use tracing::error;

mod perception;

use super::state::ServeState;

pub(crate) fn build_console_router() -> Router<ServeState> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    Router::new()
        .route("/", get(|| async { axum::response::Html(CONSOLE_HTML) }))
        .route("/health", get(health_handler))
        .route("/livez", get(live_handler))
        .route("/readyz", get(ready_handler))
        .route("/metrics", get(metrics_proxy_handler))
        .merge(perception::router())
        .layer(cors)
}

async fn health_handler(State(state): State<ServeState>) -> Json<Value> {
    let snapshot = state.health_snapshot();
    Json(json!({
        "status": "ok",
        "ready": snapshot.ready,
        "live": snapshot.live,
        "last_ready_check_ts": snapshot.last_ready_check,
        "last_error": snapshot.last_error,
    }))
}

async fn live_handler(State(state): State<ServeState>) -> impl IntoResponse {
    let snapshot = state.health_snapshot();
    let status = if snapshot.live {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "live": snapshot.live,
            "ready": snapshot.ready,
        })),
    )
}

async fn ready_handler(State(state): State<ServeState>) -> impl IntoResponse {
    let snapshot = state.health_snapshot();
    let status = if snapshot.ready {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "ready": snapshot.ready,
            "last_ready_check_ts": snapshot.last_ready_check,
            "last_error": snapshot.last_error,
        })),
    )
}

async fn metrics_proxy_handler() -> impl IntoResponse {
    metrics::register_metrics();
    let registry = metrics::global_registry();
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    if let Err(err) = encoder.encode(&registry.gather(), &mut buffer) {
        error!(?err, "failed to encode prometheus metrics for serve router");
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "metric encode error",
        )
            .into_response();
    }

    match String::from_utf8(buffer) {
        Ok(body) => match axum::http::HeaderValue::from_str(encoder.format_type()) {
            Ok(content_type) => {
                ([(axum::http::header::CONTENT_TYPE, content_type)], body).into_response()
            }
            Err(err) => {
                error!(?err, "failed to build content-type header for metrics");
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "metric encode error",
                )
                    .into_response()
            }
        },
        Err(err) => {
            error!(
                ?err,
                "failed to convert prometheus metrics to utf8 for serve router"
            );
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "metric encode error",
            )
                .into_response()
        }
    }
}
