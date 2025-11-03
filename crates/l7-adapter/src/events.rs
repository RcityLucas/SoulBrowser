use crate::ports::{ToolCall, ToolOutcome};
use l6_observe::guard::LabelMap;
use l6_observe::metrics;
use serde::Serialize;
use serde_json::Value;
use std::time::SystemTime;

/// Minimal representation of an inbound request audit event.
#[derive(Debug, Clone, Serialize, Default)]
pub struct AdapterRequestEvent {
    pub tenant_id: String,
    pub tool: String,
    pub trace_id: Option<String>,
    pub action_id: Option<String>,
    pub payload: Option<Value>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: Option<SystemTime>,
}

/// Representation of a response audit event.
#[derive(Debug, Clone, Serialize, Default)]
pub struct AdapterResponseEvent {
    pub tenant_id: String,
    pub tool: String,
    pub trace_id: Option<String>,
    pub action_id: Option<String>,
    pub latency_ms: Option<u128>,
    pub status: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: Option<SystemTime>,
}

/// Event sink interface; downstream systems can plug in metrics/audit sinks.
pub trait EventsPort: Send + Sync {
    fn on_request(&self, call: &ToolCall) {
        let event = AdapterRequestEvent {
            tenant_id: call.tenant_id.clone(),
            tool: call.tool.clone(),
            trace_id: call.trace_id.clone(),
            payload: Some(call.params.clone()),
            timestamp: Some(SystemTime::now()),
            action_id: None,
        };
        self.adapter_request(event);
    }

    fn on_response(&self, call: &ToolCall, outcome: &ToolOutcome) {
        let event = AdapterResponseEvent {
            tenant_id: call.tenant_id.clone(),
            tool: call.tool.clone(),
            trace_id: outcome.trace_id.clone(),
            action_id: outcome.action_id.clone(),
            latency_ms: None,
            status: outcome.status.clone(),
            timestamp: Some(SystemTime::now()),
        };
        self.adapter_response(event);
    }

    fn adapter_request(&self, _event: AdapterRequestEvent) {}
    fn adapter_response(&self, _event: AdapterResponseEvent) {}
}

/// Default no-op implementation used until real wiring is provided.
pub struct NoopEvents;

impl EventsPort for NoopEvents {}

/// Observer-backed implementation that emits metrics via l6-observe.
pub struct ObserverEvents;

impl ObserverEvents {
    fn base_labels(&self, tenant: &str, tool: &str) -> LabelMap {
        let mut labels = LabelMap::new();
        labels.insert("tenant".into(), tenant.to_string());
        labels.insert("tool".into(), tool.to_string());
        labels
    }
}

impl Default for ObserverEvents {
    fn default() -> Self {
        metrics::ensure_metrics();
        Self
    }
}

impl EventsPort for ObserverEvents {
    fn adapter_request(&self, event: AdapterRequestEvent) {
        let labels = self.base_labels(&event.tenant_id, &event.tool);
        metrics::inc("l7_adapter_requests_total", labels);
    }

    fn adapter_response(&self, event: AdapterResponseEvent) {
        let mut labels = self.base_labels(&event.tenant_id, &event.tool);
        labels.insert("status".into(), event.status.clone());
        metrics::inc("l7_adapter_responses_total", labels.clone());
        if let Some(latency) = event.latency_ms {
            metrics::observe("l7_adapter_response_latency_ms", latency as u64, labels);
        }
    }
}

mod time {
    pub mod serde {
        pub mod rfc3339 {
            use serde::{self, Deserialize, Deserializer, Serializer};
            use std::time::{SystemTime, UNIX_EPOCH};

            pub fn serialize<S>(
                value: &Option<SystemTime>,
                serializer: S,
            ) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                match value {
                    Some(ts) => {
                        let duration = ts
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs_f64();
                        serializer.serialize_some(&duration)
                    }
                    None => serializer.serialize_none(),
                }
            }

            #[allow(dead_code)]
            pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<SystemTime>, D::Error>
            where
                D: Deserializer<'de>,
            {
                let opt = Option::<f64>::deserialize(deserializer)?;
                Ok(opt.map(|secs| UNIX_EPOCH + std::time::Duration::from_secs_f64(secs)))
            }
        }
    }
}
