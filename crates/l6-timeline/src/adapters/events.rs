use crate::errors::TlError;
use crate::model::{QueryDigest, View};
use crate::ports::EventsPort;
use soulbrowser_event_bus::EventBus;
use std::sync::Arc;
use tokio::spawn;
use tracing::warn;

#[derive(Clone, Debug)]
pub enum TimelineRuntimeEvent {
    ExportStarted {
        view: View,
        selector_kind: String,
    },
    ExportFetched {
        count: usize,
        source: String,
    },
    ExportFinished {
        ok: bool,
        latency_ms: u128,
        error: Option<String>,
    },
}

#[derive(Default)]
pub struct NoopEventsPort;

impl EventsPort for NoopEventsPort {
    fn timeline_export_started(&self, _digest: &QueryDigest) {}

    fn timeline_export_fetched(&self, _count: usize, _source: &str) {}

    fn timeline_export_finished(&self, _ok: bool, _latency_ms: u128, _err: Option<&TlError>) {}
}

pub struct BusEventsPort {
    bus: Arc<dyn EventBus<TimelineRuntimeEvent> + Send + Sync>,
}

impl BusEventsPort {
    pub fn new(bus: Arc<dyn EventBus<TimelineRuntimeEvent> + Send + Sync>) -> Self {
        Self { bus }
    }

    fn publish(&self, event: TimelineRuntimeEvent) {
        let bus = Arc::clone(&self.bus);
        spawn(async move {
            if let Err(err) = bus.publish(event).await {
                warn!(?err, "timeline event bus publish failed");
            }
        });
    }
}

impl EventsPort for BusEventsPort {
    fn timeline_export_started(&self, digest: &QueryDigest) {
        self.publish(TimelineRuntimeEvent::ExportStarted {
            view: digest.view.clone(),
            selector_kind: digest.selector_kind.to_string(),
        });
    }

    fn timeline_export_fetched(&self, count: usize, source: &str) {
        self.publish(TimelineRuntimeEvent::ExportFetched {
            count,
            source: source.to_string(),
        });
    }

    fn timeline_export_finished(&self, ok: bool, latency_ms: u128, err: Option<&TlError>) {
        self.publish(TimelineRuntimeEvent::ExportFinished {
            ok,
            latency_ms,
            error: err.map(|e| e.to_string()),
        });
    }
}
