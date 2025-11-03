use crate::errors::{AdapterError, AdapterResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use soulbrowser_scheduler::model::SubmitHandle;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ToolCall {
    pub tenant_id: String,
    pub tool: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub routing: Value,
    #[serde(default)]
    pub options: Value,
    pub timeout_ms: u64,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ToolOutcome {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TimelineExportReq {
    pub tenant_id: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TimelineExportOutcome {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

#[async_trait]
pub trait DispatcherPort: Send + Sync {
    async fn run_tool(&self, call: ToolCall) -> AdapterResult<ToolOutcome>;
}

#[async_trait]
pub trait ReadonlyPort: Send + Sync {
    async fn export_timeline(&self, req: TimelineExportReq)
        -> AdapterResult<TimelineExportOutcome>;
}

pub struct NoopDispatcher;

#[async_trait]
impl DispatcherPort for NoopDispatcher {
    async fn run_tool(&self, _call: ToolCall) -> AdapterResult<ToolOutcome> {
        Err(AdapterError::NotImplemented("dispatcher"))
    }
}

pub struct NoopReadonly;

#[async_trait]
impl ReadonlyPort for NoopReadonly {
    async fn export_timeline(
        &self,
        _req: TimelineExportReq,
    ) -> AdapterResult<TimelineExportOutcome> {
        Err(AdapterError::NotImplemented("readonly"))
    }
}

/// Bridge to the L1 scheduler dispatcher; currently returns a placeholder
/// outcome until full wiring to L5 stack is landed.
pub struct SchedulerDispatcher<D> {
    inner: D,
}

impl<D> SchedulerDispatcher<D> {
    pub fn new(inner: D) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<D> DispatcherPort for SchedulerDispatcher<D>
where
    D: soulbrowser_scheduler::Dispatcher + Send + Sync,
{
    async fn run_tool(&self, call: ToolCall) -> AdapterResult<ToolOutcome> {
        let dispatch = crate::map::to_dispatch_request(&call)?;
        let SubmitHandle {
            action_id,
            receiver,
        } = self
            .inner
            .submit(dispatch)
            .await
            .map_err(|_| AdapterError::Internal)?;

        let output = receiver.await.map_err(|_| AdapterError::Internal)?;
        let status = if output.error.is_some() {
            "error"
        } else {
            "ok"
        };

        let queue_ms = output.timeline.started_at.map(|started| {
            duration_ms(started.saturating_duration_since(output.timeline.enqueued_at))
        });
        let run_ms = output
            .timeline
            .finished_at
            .zip(output.timeline.started_at)
            .map(|(finish, start)| duration_ms(finish.saturating_duration_since(start)));

        let mut data = serde_json::json!({
            "route": {
                "session": output.route.session.0,
                "page": output.route.page.0,
                "frame": output.route.frame.0,
            },
            "timeline": {
                "queue_ms": queue_ms,
                "run_ms": run_ms,
            }
        });

        if let Some(error) = output.error {
            if let Some(obj) = data.as_object_mut() {
                obj.insert(
                    "error".into(),
                    serde_json::json!({
                        "message": error.to_string(),
                    }),
                );
            }
        }

        Ok(ToolOutcome {
            status: status.into(),
            data: Some(data),
            trace_id: call.trace_id.clone(),
            action_id: Some(action_id.0),
        })
    }
}

fn duration_ms(duration: std::time::Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use soulbrowser_core_types::{ActionId, ExecRoute, FrameId, PageId, SessionId, SoulError};
    use soulbrowser_scheduler::model::{
        DispatchOutput, DispatchRequest, DispatchTimeline, SubmitHandle,
    };
    use soulbrowser_scheduler::Dispatcher;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    use tokio::sync::oneshot;

    struct StubDispatcher {
        record: Arc<Mutex<Option<DispatchRequest>>>,
    }

    #[async_trait]
    impl Dispatcher for StubDispatcher {
        async fn submit(&self, call: DispatchRequest) -> Result<SubmitHandle, SoulError> {
            *self.record.lock().unwrap() = Some(call.clone());
            let (tx, rx) = oneshot::channel();
            let route = ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new());
            let timeline = DispatchTimeline {
                enqueued_at: Instant::now(),
                started_at: Some(Instant::now()),
                finished_at: Some(Instant::now()),
            };
            tx.send(DispatchOutput::ok(route, timeline, None)).unwrap();
            Ok(SubmitHandle {
                action_id: ActionId::new(),
                receiver: rx,
            })
        }

        async fn cancel(&self, _action: ActionId) -> Result<bool, SoulError> {
            Ok(false)
        }

        async fn cancel_call(&self, _call_id: &str) -> Result<bool, SoulError> {
            Ok(false)
        }

        async fn cancel_task(&self, _task_id: &str) -> Result<usize, SoulError> {
            Ok(0)
        }
    }

    #[tokio::test]
    async fn scheduler_dispatcher_maps_call() {
        let record = Arc::new(Mutex::new(None));
        let dispatcher = SchedulerDispatcher::new(StubDispatcher {
            record: record.clone(),
        });
        let call = ToolCall {
            tenant_id: "tenant-1".into(),
            tool: "click".into(),
            params: serde_json::json!({ "selector": "#btn" }),
            routing: serde_json::json!({ "session": "s-1" }),
            options: serde_json::json!({ "priority": "quick" }),
            timeout_ms: 5000,
            idempotency_key: Some("demo".into()),
            trace_id: Some("trace-1".into()),
        };
        let outcome = dispatcher.run_tool(call.clone()).await.unwrap();
        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.trace_id.as_deref(), Some("trace-1"));
        assert!(outcome.action_id.is_some());
        let recorded = record.lock().unwrap().clone().unwrap();
        assert_eq!(recorded.tool_call.tool, "click");
        assert_eq!(recorded.options.timeout.as_millis(), 5000);
        assert!(recorded.routing_hint.is_some());
        let data = outcome.data.expect("data expected");
        assert!(data["timeline"]["run_ms"].is_number());
    }
}
