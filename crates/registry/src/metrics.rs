use lazy_static::lazy_static;
use network_tap_light::NetworkSnapshot;
use prometheus::{core::Collector, opts, IntCounter, IntCounterVec, IntGauge, Registry};
use tracing::error;

lazy_static! {
    static ref REGISTRY_SESSIONS_TOTAL: IntGauge =
        IntGauge::new("soul_registry_sessions_total", "Total active sessions").unwrap();
    static ref REGISTRY_PAGES_TOTAL: IntGauge =
        IntGauge::new("soul_registry_pages_total", "Total active pages").unwrap();
    static ref REGISTRY_FRAMES_TOTAL: IntGauge =
        IntGauge::new("soul_registry_frames_total", "Total active frames").unwrap();
    static ref REGISTRY_PAGE_HEALTH_UPDATES: IntCounter = IntCounter::new(
        "soul_registry_page_health_updates_total",
        "Page health updates recorded",
    )
    .unwrap();
    static ref REGISTRY_PERMISSIONS_APPLIED: IntCounterVec = IntCounterVec::new(
        opts!(
            "soul_registry_permissions_applied_total",
            "Permissions applied grouped by origin"
        ),
        &["origin"]
    )
    .unwrap();
}

fn register<C>(registry: &Registry, collector: C)
where
    C: Collector + Clone + Send + Sync + 'static,
{
    if let Err(err) = registry.register(Box::new(collector.clone())) {
        if !matches!(err, prometheus::Error::AlreadyReg) {
            error!(?err, "failed to register registry metric");
        }
    }
}

pub fn register_metrics(registry: &Registry) {
    register(registry, REGISTRY_SESSIONS_TOTAL.clone());
    register(registry, REGISTRY_PAGES_TOTAL.clone());
    register(registry, REGISTRY_FRAMES_TOTAL.clone());
    register(registry, REGISTRY_PAGE_HEALTH_UPDATES.clone());
    register(registry, REGISTRY_PERMISSIONS_APPLIED.clone());
}

pub fn set_session_count(count: usize) {
    REGISTRY_SESSIONS_TOTAL.set(count as i64);
}

pub fn set_page_count(count: usize) {
    REGISTRY_PAGES_TOTAL.set(count as i64);
}

pub fn set_frame_count(count: usize) {
    REGISTRY_FRAMES_TOTAL.set(count as i64);
}

pub fn record_page_health_update(_snapshot: &NetworkSnapshot) {
    REGISTRY_PAGE_HEALTH_UPDATES.inc();
}

pub fn record_permissions_applied(origin: &str) {
    REGISTRY_PERMISSIONS_APPLIED
        .with_label_values(&[origin])
        .inc();
}
