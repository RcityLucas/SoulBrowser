use std::sync::atomic::{AtomicU64, Ordering};
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
use tracing::{debug, error, info, warn};

static GLOBAL_REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);
static REGISTER_ONCE: OnceCell<()> = OnceCell::new();
static EXECUTION_STEP_LATENCY: OnceCell<HistogramVec> = OnceCell::new();
static EXECUTION_STEP_ATTEMPTS: OnceCell<IntCounterVec> = OnceCell::new();
static LLM_CACHE_EVENTS: OnceCell<IntCounterVec> = OnceCell::new();
static EXECUTION_MISSING_RESULT: OnceCell<IntCounterVec> = OnceCell::new();
static PLAN_REJECTIONS: OnceCell<IntCounterVec> = OnceCell::new();
static PLAN_TEMPLATE_USAGE: OnceCell<IntCounterVec> = OnceCell::new();
static PLAN_STRATEGY_USAGE: OnceCell<IntCounterVec> = OnceCell::new();
static PLAN_AUTO_REPAIRS: OnceCell<IntCounterVec> = OnceCell::new();
static GUARDRAIL_KEYWORD_USAGE: OnceCell<IntCounterVec> = OnceCell::new();
static AUTO_ACT_SEARCH_ENGINES: OnceCell<IntCounterVec> = OnceCell::new();
static MARKET_QUOTE_FETCH: OnceCell<IntCounterVec> = OnceCell::new();
static MARKET_QUOTE_FALLBACK: OnceCell<IntCounterVec> = OnceCell::new();
static MANUAL_TAKEOVER: OnceCell<IntCounterVec> = OnceCell::new();
static MISSING_RESULT_TOTAL: AtomicU64 = AtomicU64::new(0);

const MISSING_RESULT_ALERT_THRESHOLD: u64 = 10;

pub fn register_metrics() {
    REGISTER_ONCE.get_or_init(|| {
        let registry = global_registry();
        scheduler_metrics::register_metrics(registry);
        registry_metrics::register_metrics(registry);
        cdp_metrics::register_metrics(registry);
        register_execution_metrics(registry);
        register_llm_cache_metrics(registry);
        register_plan_metrics(registry);
        register_quote_metrics(registry);
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

    let missing = IntCounterVec::new(
        Opts::new(
            "soul_execution_missing_result_total",
            "Executions that finished without user-facing results",
        ),
        &["intent_kind"],
    )
    .expect("create missing result counter");
    if let Err(err) = registry.register(Box::new(missing.clone())) {
        error!(?err, "failed to register missing result counter");
    }
    let _ = EXECUTION_MISSING_RESULT.set(missing);
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

fn register_plan_metrics(registry: &Registry) {
    let rejections = IntCounterVec::new(
        Opts::new(
            "soul_plan_rejections_total",
            "Plans rejected by validation grouped by reason",
        ),
        &["reason"],
    )
    .expect("create plan rejection counter");
    if let Err(err) = registry.register(Box::new(rejections.clone())) {
        error!(?err, "failed to register plan rejection counter");
    }
    let _ = PLAN_REJECTIONS.set(rejections);

    let templates = IntCounterVec::new(
        Opts::new(
            "soul_plan_template_usage_total",
            "Planner intent templates applied grouped by recipe",
        ),
        &["template"],
    )
    .expect("create template usage counter");
    if let Err(err) = registry.register(Box::new(templates.clone())) {
        error!(?err, "failed to register template usage counter");
    }
    let _ = PLAN_TEMPLATE_USAGE.set(templates);

    let strategies = IntCounterVec::new(
        Opts::new(
            "soul_plan_strategy_usage_total",
            "Stage strategy attempts grouped by stage and outcome",
        ),
        &["stage", "strategy", "result"],
    )
    .expect("create strategy usage counter");
    if let Err(err) = registry.register(Box::new(strategies.clone())) {
        error!(?err, "failed to register strategy usage counter");
    }
    let _ = PLAN_STRATEGY_USAGE.set(strategies);

    let auto_repairs = IntCounterVec::new(
        Opts::new(
            "soul_plan_auto_repairs_total",
            "Auto repair events aggregated by kind",
        ),
        &["kind"],
    )
    .expect("create auto repair counter");
    if let Err(err) = registry.register(Box::new(auto_repairs.clone())) {
        error!(?err, "failed to register auto repair counter");
    }
    let _ = PLAN_AUTO_REPAIRS.set(auto_repairs);

    let guardrail_keywords = IntCounterVec::new(
        Opts::new(
            "soul_guardrail_keyword_seeds_total",
            "Guardrail keywords merged into search seeds grouped by intent",
        ),
        &["intent"],
    )
    .expect("create guardrail keyword counter");
    if let Err(err) = registry.register(Box::new(guardrail_keywords.clone())) {
        error!(?err, "failed to register guardrail keyword counter");
    }
    let _ = GUARDRAIL_KEYWORD_USAGE.set(guardrail_keywords);

    let auto_act = IntCounterVec::new(
        Opts::new(
            "soul_auto_act_search_engine_total",
            "AutoAct search submissions grouped by intent and engine",
        ),
        &["intent", "engine"],
    )
    .expect("create auto act search counter");
    if let Err(err) = registry.register(Box::new(auto_act.clone())) {
        error!(?err, "failed to register auto act search counter");
    }
    let _ = AUTO_ACT_SEARCH_ENGINES.set(auto_act);
}

fn register_quote_metrics(registry: &Registry) {
    let quote_fetch = IntCounterVec::new(
        Opts::new(
            "soul_market_quote_fetch_total",
            "Market quote fetch attempts grouped by mode/result",
        ),
        &["mode", "result"],
    )
    .expect("create market quote fetch counter");
    if let Err(err) = registry.register(Box::new(quote_fetch.clone())) {
        error!(?err, "failed to register quote fetch metrics");
    }
    let _ = MARKET_QUOTE_FETCH.set(quote_fetch);

    let fallback = IntCounterVec::new(
        Opts::new(
            "soul_market_quote_fallback_total",
            "Market quote fallback attempts grouped by kind",
        ),
        &["kind"],
    )
    .expect("create market quote fallback counter");
    if let Err(err) = registry.register(Box::new(fallback.clone())) {
        error!(?err, "failed to register quote fallback metrics");
    }
    let _ = MARKET_QUOTE_FALLBACK.set(fallback);

    let takeover = IntCounterVec::new(
        Opts::new(
            "soul_manual_takeover_total",
            "Manual takeover triggers grouped by source",
        ),
        &["source"],
    )
    .expect("create manual takeover metric");
    if let Err(err) = registry.register(Box::new(takeover.clone())) {
        error!(?err, "failed to register manual takeover metric");
    }
    let _ = MANUAL_TAKEOVER.set(takeover);
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

pub fn record_plan_rejection(reason: &str) {
    register_metrics();
    if let Some(counter) = PLAN_REJECTIONS.get() {
        counter.with_label_values(&[reason]).inc();
    }
    debug!(target = "plan_validation", %reason, "plan rejected");
}

pub fn record_guardrail_keyword_usage(intent: &str, count: usize) {
    if count == 0 {
        return;
    }
    if let Some(counter) = GUARDRAIL_KEYWORD_USAGE.get() {
        counter.with_label_values(&[intent]).inc_by(count as u64);
    }
}

pub fn record_auto_act_search_engine(intent: &str, engine: &str) {
    if let Some(counter) = AUTO_ACT_SEARCH_ENGINES.get() {
        counter.with_label_values(&[intent, engine]).inc();
    }
}

pub fn record_template_usage(template: &str) {
    register_metrics();
    if let Some(counter) = PLAN_TEMPLATE_USAGE.get() {
        counter.with_label_values(&[template]).inc();
    }
    debug!(target = "planner", %template, "template recipe applied");
}

pub fn record_strategy_usage(stage: &str, strategy: &str, result: &str) {
    register_metrics();
    if let Some(counter) = PLAN_STRATEGY_USAGE.get() {
        counter.with_label_values(&[stage, strategy, result]).inc();
    }
    debug!(target = "planner", %stage, %strategy, %result, "stage strategy usage");
}

pub fn record_auto_repair_events(kind: &str, count: u64) {
    if count == 0 {
        return;
    }
    register_metrics();
    if let Some(counter) = PLAN_AUTO_REPAIRS.get() {
        counter.with_label_values(&[kind]).inc_by(count);
    }
    debug!(target = "planner", %kind, count, "auto repair recorded");
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

pub fn record_missing_user_result(intent_kind: &str) {
    register_metrics();
    if let Some(counter) = EXECUTION_MISSING_RESULT.get() {
        counter.with_label_values(&[intent_kind]).inc();
    }
    let total = MISSING_RESULT_TOTAL.fetch_add(1, Ordering::Relaxed) + 1;
    if total % MISSING_RESULT_ALERT_THRESHOLD == 0 {
        warn!(
            target = "execution",
            total,
            %intent_kind,
            "executions succeeded without user-facing results"
        );
    } else {
        debug!(target = "execution", %intent_kind, "execution missing user result");
    }
}

pub fn record_market_quote_fetch(mode: &str, result: &str) {
    register_metrics();
    if let Some(counter) = MARKET_QUOTE_FETCH.get() {
        counter.with_label_values(&[mode, result]).inc();
    }
    debug!(target = "quotes", %mode, %result, "market quote fetch attempt");
}

pub fn record_market_quote_fallback(kind: &str) {
    register_metrics();
    if let Some(counter) = MARKET_QUOTE_FALLBACK.get() {
        counter.with_label_values(&[kind]).inc();
    }
    debug!(target = "quotes", %kind, "market quote fallback");
}

pub fn record_manual_takeover_triggered(source: &str) {
    register_metrics();
    if let Some(counter) = MANUAL_TAKEOVER.get() {
        counter.with_label_values(&[source]).inc();
    }
    debug!(target = "manual_override", %source, "manual takeover triggered");
}
