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
use prometheus::{
    histogram_opts, Encoder, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder,
};
use soulbrowser_registry::metrics as registry_metrics;
use soulbrowser_scheduler::metrics as scheduler_metrics;
use tokio::{net::TcpListener, task::JoinHandle};
use tracing::{debug, error, info};

static GLOBAL_REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);
static REGISTER_ONCE: OnceCell<()> = OnceCell::new();
static EXECUTION_STEP_LATENCY: OnceCell<HistogramVec> = OnceCell::new();
static EXECUTION_STEP_ATTEMPTS: OnceCell<IntCounterVec> = OnceCell::new();
static LLM_CACHE_EVENTS: OnceCell<IntCounterVec> = OnceCell::new();

pub fn register_metrics() {
    REGISTER_ONCE.get_or_init(|| {
        let registry = global_registry();
        scheduler_metrics::register_metrics(registry);
        registry_metrics::register_metrics(registry);
        cdp_metrics::register_metrics(registry);
        register_execution_metrics(registry);
        register_llm_cache_metrics(registry);
    });
}

fn register_execution_metrics(registry: &Registry) {
    let latency = HistogramVec::new(
        histogram_opts!(
            "soul_execution_step_latency_ms",
            "Aggregated wait/run latency per agent step (milliseconds)",
            vec![5.0, 10.0, 20.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0]
        ),
        &["phase", "tool", "result"],
    )
    .expect("create execution latency histogram");
    if let Err(err) = registry.register(Box::new(latency.clone())) {
        error!(?err, "failed to register execution latency histogram");
    }
    let _ = EXECUTION_STEP_LATENCY.set(latency);

    let attempts = IntCounterVec::new(
        Opts::new(
            "soul_execution_step_attempts_total",
            "Total execution attempts recorded per agent step",
        ),
        &["tool", "result"],
    )
    .expect("create execution attempts counter");
    if let Err(err) = registry.register(Box::new(attempts.clone())) {
        error!(?err, "failed to register execution attempts counter");
    }
    let _ = EXECUTION_STEP_ATTEMPTS.set(attempts);
}

fn register_llm_cache_metrics(registry: &Registry) {
    let events = IntCounterVec::new(
        Opts::new(
            "soul_llm_cache_events_total",
            "LLM cache operations by namespace/event",
        ),
        &["namespace", "event"],
    )
    .expect("create llm cache counter");
    if let Err(err) = registry.register(Box::new(events.clone())) {
        error!(?err, "failed to register llm cache metrics");
    }
    let _ = LLM_CACHE_EVENTS.set(events);
}

pub fn observe_execution_step(tool: &str, result: &str, wait_ms: u64, run_ms: u64, attempts: u64) {
    register_metrics();
    if let Some(histogram) = EXECUTION_STEP_LATENCY.get() {
        let wait = wait_ms as f64;
        let run = run_ms as f64;
        histogram
            .with_label_values(&["wait", tool, result])
            .observe(wait);
        histogram
            .with_label_values(&["run", tool, result])
            .observe(run);
    }
    if let Some(counter) = EXECUTION_STEP_ATTEMPTS.get() {
        counter.with_label_values(&[tool, result]).inc_by(attempts);
    }
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

pub fn record_llm_cache_event(namespace: &str, event: &str) {
    register_metrics();
    if let Some(counter) = LLM_CACHE_EVENTS.get() {
        counter.with_label_values(&[namespace, event]).inc();
    }
    debug!(target = "llm_cache", %namespace, %event, "llm cache metric");
}

pub fn record_watchdog_event(kind: &str) {
    debug!(target = "watchdog", %kind, "watchdog event");
}

pub fn record_permission_prompt() {
    debug!(target = "watchdog", "permission prompt detected");
}

pub fn record_download_prompt() {
    debug!(target = "watchdog", "download prompt detected");
}
