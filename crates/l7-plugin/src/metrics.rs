use once_cell::sync::Lazy;
use prometheus::{register_histogram_vec, register_int_counter_vec, HistogramVec, IntCounterVec};

pub static PLUGIN_CALLS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "l7_plugin_calls_total",
        "Number of plugin hook executions",
        &["plugin", "hook", "result"]
    )
    .expect("register l7_plugin_calls_total")
});

pub static PLUGIN_CALL_LATENCY: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "l7_plugin_call_latency_ms",
        "Latency of plugin hook execution",
        &["plugin", "hook"]
    )
    .expect("register l7_plugin_call_latency_ms")
});

pub fn observe_call(plugin: &str, hook: &str, ok: bool, latency_ms: f64) {
    let result = if ok { "ok" } else { "error" };
    PLUGIN_CALLS_TOTAL
        .with_label_values(&[plugin, hook, result])
        .inc();
    PLUGIN_CALL_LATENCY
        .with_label_values(&[plugin, hook])
        .observe(latency_ms);
}
