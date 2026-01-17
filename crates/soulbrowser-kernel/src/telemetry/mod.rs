use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::Serialize;
use serde_json::{json, Value};
use soulbrowser_core_types::TaskId;
use std::env;
use std::sync::{Arc, Once};
use tokio::sync::broadcast;

use crate::agent::executor::StepExecutionReport;

static TELEMETRY_REGISTRY: Lazy<TelemetryHub> = Lazy::new(TelemetryHub::default);
static CONFIGURE_ONCE: Once = Once::new();

pub fn configure_from_env() {
    CONFIGURE_ONCE.call_once(|| {
        if parse_env_flag("SOULBROWSER_TELEMETRY_STDOUT") {
            TELEMETRY_REGISTRY.register_sink(Arc::new(StdoutTelemetrySink));
        }
    });
}

pub fn register_sink(sink: Box<dyn TelemetrySink>) {
    TELEMETRY_REGISTRY.register_sink(Arc::from(sink));
}

pub fn emit_step_report(tenant: &str, task_id: &TaskId, report: &StepExecutionReport) {
    let event = TelemetryEvent::from_step(tenant, task_id, report);
    TELEMETRY_REGISTRY.emit(event);
}

pub fn subscribe() -> broadcast::Receiver<TelemetryEvent> {
    TELEMETRY_REGISTRY.subscribe()
}

fn parse_env_flag(name: &str) -> bool {
    matches!(
        env::var(name)
            .ok()
            .map(|value| value.to_ascii_lowercase()),
        Some(flag) if flag == "1" || flag == "true" || flag == "yes"
    )
}

pub struct TelemetryHub {
    sinks: RwLock<Vec<Arc<dyn TelemetrySink>>>,
    channel: broadcast::Sender<TelemetryEvent>,
}

impl Default for TelemetryHub {
    fn default() -> Self {
        let (tx, _rx) = broadcast::channel(512);
        Self {
            sinks: RwLock::new(Vec::new()),
            channel: tx,
        }
    }
}

impl TelemetryHub {
    pub fn register_sink(&self, sink: Arc<dyn TelemetrySink>) {
        self.sinks.write().push(sink);
    }

    pub fn emit(&self, event: TelemetryEvent) {
        let sinks: Vec<Arc<dyn TelemetrySink>> = self.sinks.read().iter().cloned().collect();
        for sink in sinks {
            sink.emit(&event);
        }
        let _ = self.channel.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TelemetryEvent> {
        self.channel.subscribe()
    }
}

pub trait TelemetrySink: Send + Sync {
    fn emit(&self, event: &TelemetryEvent);
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetryEvent {
    pub timestamp: DateTime<Utc>,
    pub tenant: String,
    pub task_id: String,
    pub kind: TelemetryEventKind,
    pub payload: Value,
    pub metrics: TelemetryMetrics,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TelemetryMetrics {
    pub llm_input_tokens: Option<u64>,
    pub llm_output_tokens: Option<u64>,
    pub runtime_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryEventKind {
    StepCompleted,
    StepFailed,
}

impl TelemetryEvent {
    pub fn from_step(tenant: &str, task_id: &TaskId, report: &StepExecutionReport) -> Self {
        let kind = match report.status {
            crate::agent::StepExecutionStatus::Success => TelemetryEventKind::StepCompleted,
            crate::agent::StepExecutionStatus::Failed => TelemetryEventKind::StepFailed,
        };
        let payload = json!({
            "step_id": report.step_id,
            "title": report.title,
            "tool": report.tool_kind,
            "status": match report.status {
                crate::agent::StepExecutionStatus::Success => "success",
                crate::agent::StepExecutionStatus::Failed => "failed",
            },
            "attempts": report.attempts,
            "total_run_ms": report.total_run_ms,
            "total_wait_ms": report.total_wait_ms,
            "blocker_kind": report.blocker_kind,
            "observation_summary": report.observation_summary,
        });
        Self {
            timestamp: Utc::now(),
            tenant: tenant.to_string(),
            task_id: task_id.0.clone(),
            kind,
            payload,
            metrics: TelemetryMetrics {
                llm_input_tokens: report
                    .agent_state
                    .as_ref()
                    .and_then(|state| state.get("llm_input_tokens").and_then(Value::as_u64)),
                llm_output_tokens: report
                    .agent_state
                    .as_ref()
                    .and_then(|state| state.get("llm_output_tokens").and_then(Value::as_u64)),
                runtime_ms: Some(report.total_run_ms),
            },
        }
    }
}

#[derive(Default)]
struct StdoutTelemetrySink;

impl TelemetrySink for StdoutTelemetrySink {
    fn emit(&self, event: &TelemetryEvent) {
        if let Ok(line) = serde_json::to_string(event) {
            println!("[telemetry] {}", line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::StepExecutionStatus;

    struct TestSink {
        events: Arc<RwLock<Vec<TelemetryEvent>>>,
    }

    impl TestSink {
        fn new(store: Arc<RwLock<Vec<TelemetryEvent>>>) -> Self {
            Self { events: store }
        }
    }

    impl TelemetrySink for TestSink {
        fn emit(&self, event: &TelemetryEvent) {
            self.events.write().push(event.clone());
        }
    }

    fn sample_report() -> StepExecutionReport {
        StepExecutionReport {
            step_id: "llm-step-1".to_string(),
            title: "Navigate".to_string(),
            tool_kind: "navigate-to-url".to_string(),
            status: StepExecutionStatus::Success,
            attempts: 1,
            error: None,
            dispatches: Vec::new(),
            total_wait_ms: 100,
            total_run_ms: 1500,
            observation_summary: Some("Loaded homepage".to_string()),
            blocker_kind: None,
            agent_state: None,
        }
    }

    #[test]
    fn emits_step_events_to_registered_sink() {
        let hub = TelemetryHub::default();
        let events = Arc::new(RwLock::new(Vec::new()));
        hub.register_sink(Arc::new(TestSink::new(events.clone())));
        let report = sample_report();
        let event = TelemetryEvent::from_step("tenant", &TaskId("task".into()), &report);
        hub.emit(event);
        let stored = events.read();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].payload["step_id"], "llm-step-1");
    }
}
