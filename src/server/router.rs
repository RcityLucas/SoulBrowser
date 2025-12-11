use axum::{extract::State, http::Method, response::IntoResponse, routing::get, Json, Router};
use serde_json::{json, Value};
use tower_http::cors::{Any, CorsLayer};

use crate::{metrics, CONSOLE_HTML};
use prometheus::{Encoder, TextEncoder};
use tracing::error;

mod admin;
mod chat;
mod memory;
mod perception;
mod plugins;
mod self_heal;
mod tasks;
mod ws;

pub(crate) use admin::router as admin_routes;
pub(crate) use chat::router as chat_routes;
pub(crate) use memory::router as memory_routes;
pub(crate) use perception::router as perception_routes;
pub(crate) use plugins::router as plugin_routes;
pub(crate) use self_heal::router as self_heal_routes;
pub(crate) use tasks::router as task_routes;

use super::state::ServeState;

pub(crate) fn build_console_router() -> Router<ServeState> {
    build_console_router_with_modules(ServeRouterModules::all())
}

pub(crate) fn build_console_router_with_modules(modules: ServeRouterModules) -> Router<ServeState> {
    console_shell_router().merge(build_api_router_with_modules(modules))
}

pub(crate) fn console_shell_router() -> Router<ServeState> {
    Router::new()
        .route("/", get(|| async { axum::response::Html(CONSOLE_HTML) }))
        .route("/health", get(health_handler))
        .route("/livez", get(live_handler))
        .route("/readyz", get(ready_handler))
        .route("/metrics", get(metrics_proxy_handler))
        .merge(ws::router())
        .layer(cors_layer())
}

pub(crate) fn build_api_router_with_modules(modules: ServeRouterModules) -> Router<ServeState> {
    let router = Router::new();
    modules.apply(router).layer(cors_layer())
}

fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ServeRouterModules {
    pub perception: bool,
    pub chat: bool,
    pub tasks: bool,
    pub memory: bool,
    pub plugins: bool,
    pub self_heal: bool,
    pub admin: bool,
}

impl ServeRouterModules {
    pub const fn all() -> Self {
        Self {
            perception: true,
            chat: true,
            tasks: true,
            memory: true,
            plugins: true,
            self_heal: true,
            admin: true,
        }
    }

    pub const fn none() -> Self {
        Self {
            perception: false,
            chat: false,
            tasks: false,
            memory: false,
            plugins: false,
            self_heal: false,
            admin: false,
        }
    }

    pub const fn with_perception(mut self, enabled: bool) -> Self {
        self.perception = enabled;
        self
    }

    pub const fn with_chat(mut self, enabled: bool) -> Self {
        self.chat = enabled;
        self
    }

    pub const fn with_tasks(mut self, enabled: bool) -> Self {
        self.tasks = enabled;
        self
    }

    pub const fn with_memory(mut self, enabled: bool) -> Self {
        self.memory = enabled;
        self
    }

    pub const fn with_plugins(mut self, enabled: bool) -> Self {
        self.plugins = enabled;
        self
    }

    pub const fn with_self_heal(mut self, enabled: bool) -> Self {
        self.self_heal = enabled;
        self
    }

    pub const fn with_admin(mut self, enabled: bool) -> Self {
        self.admin = enabled;
        self
    }

    fn apply(self, mut router: Router<ServeState>) -> Router<ServeState> {
        if self.perception {
            router = router.merge(perception_routes());
        }
        if self.chat {
            router = router.merge(chat_routes());
        }
        if self.tasks {
            router = router.merge(task_routes());
        }
        if self.memory {
            router = router.merge(memory_routes());
        }
        if self.plugins {
            router = router.merge(plugin_routes());
        }
        if self.self_heal {
            router = router.merge(self_heal_routes());
        }
        if self.admin {
            router = router.merge(admin_routes());
        }

        router
    }
}

impl Default for ServeRouterModules {
    fn default() -> Self {
        Self::all()
    }
}

async fn health_handler(State(state): State<ServeState>) -> Json<Value> {
    let snapshot = state.health_snapshot();
    Json(json!({
        "status": "ok",
        "pooling_enabled": snapshot.pooling_enabled,
        "pooling_cooldown_secs": snapshot.pooling_cooldown_secs,
        "llm_cache": snapshot.llm_cache_enabled,
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
            "pooling_enabled": snapshot.pooling_enabled,
            "pooling_cooldown_secs": snapshot.pooling_cooldown_secs,
            "llm_cache": snapshot.llm_cache_enabled,
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
