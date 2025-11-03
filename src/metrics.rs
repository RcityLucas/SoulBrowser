use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::State,
    http::HeaderValue,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use cdp_adapter::metrics as cdp_metrics;
use once_cell::sync::{Lazy, OnceCell};
use prometheus::{Encoder, Registry, TextEncoder};
use soulbrowser_registry::metrics as registry_metrics;
use soulbrowser_scheduler::metrics as scheduler_metrics;
use tokio::{net::TcpListener, task::JoinHandle};
use tracing::{error, info};

static GLOBAL_REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);
static REGISTER_ONCE: OnceCell<()> = OnceCell::new();

pub fn register_metrics() {
    REGISTER_ONCE.get_or_init(|| {
        let registry = global_registry();
        scheduler_metrics::register_metrics(registry);
        registry_metrics::register_metrics(registry);
        cdp_metrics::register_metrics(registry);
    });
}

pub fn spawn_metrics_server(port: u16) -> Option<JoinHandle<()>> {
    if port == 0 {
        return None;
    }

    register_metrics();
    let registry = Arc::new(global_registry().clone());
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(registry);

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    info!(%addr, "metrics server listening");
    Some(tokio::spawn(async move {
        match TcpListener::bind(addr).await {
            Ok(listener) => {
                if let Err(err) = axum::serve(listener, app.into_make_service()).await {
                    error!(?err, "metrics server exited with error");
                }
            }
            Err(err) => {
                error!(?err, "failed to bind metrics listener");
            }
        }
    }))
}

async fn metrics_handler(State(registry): State<Arc<Registry>>) -> Response {
    let encoder = TextEncoder::new();
    let format_type = encoder.format_type().to_string();
    let metric_families = registry.gather();
    let mut buffer = Vec::new();
    if let Err(err) = encoder.encode(&metric_families, &mut buffer) {
        error!(?err, "failed to encode prometheus metrics");
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "metric encode error",
        )
            .into_response();
    }

    match String::from_utf8(buffer) {
        Ok(body) => match HeaderValue::from_str(&format_type) {
            Ok(value) => ([(axum::http::header::CONTENT_TYPE, value)], body).into_response(),
            Err(err) => {
                error!(?err, "failed to build content-type header");
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "metric encode error",
                )
                    .into_response()
            }
        },
        Err(err) => {
            error!(?err, "failed to convert prometheus metrics to utf8");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "metric encode error",
            )
                .into_response()
        }
    }
}

pub fn global_registry() -> &'static Registry {
    &GLOBAL_REGISTRY
}
