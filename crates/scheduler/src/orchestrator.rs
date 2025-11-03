use std::{sync::Arc, time::Instant};

use soulbrowser_core_types::{ActionId, ExecRoute, SoulError};
use soulbrowser_registry::Registry;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

use crate::executor::ToolExecutor;
use crate::metrics;
use crate::model::{DispatchOutput, DispatchRequest, DispatchTimeline, SubmitHandle};
use crate::runtime::{ReadyJob, SchedulerRuntime};
use serde_json::Value;
use soulbrowser_state_center::{DispatchEvent, StateCenter, StateEvent};

pub struct Orchestrator<R, E>
where
    R: Registry + Send + Sync + 'static,
    E: ToolExecutor + Send + Sync + 'static,
{
    registry: Arc<R>,
    runtime: Arc<SchedulerRuntime>,
    executor: Arc<E>,
    state_center: Arc<dyn StateCenter>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl<R, E> Orchestrator<R, E>
where
    R: Registry + Send + Sync + 'static,
    E: ToolExecutor + Send + Sync + 'static,
{
    pub fn new(
        registry: Arc<R>,
        runtime: Arc<SchedulerRuntime>,
        executor: Arc<E>,
        state_center: Arc<dyn StateCenter>,
    ) -> Self {
        Self {
            registry,
            runtime,
            executor,
            state_center,
            worker: Mutex::new(None),
        }
    }

    pub async fn spawn(&self) {
        let mut guard = self.worker.lock().await;
        if guard.is_some() {
            return;
        }
        let runtime = Arc::clone(&self.runtime);
        let registry = Arc::clone(&self.registry);
        let executor = Arc::clone(&self.executor);
        let state_center = Arc::clone(&self.state_center);
        let handle = tokio::spawn(async move {
            loop {
                match runtime.next_job().await {
                    Some(job) => {
                        if let Err(err) =
                            dispatch_job(&registry, &runtime, &executor, &state_center, job).await
                        {
                            warn!("scheduler dispatch failed: {err}");
                        }
                    }
                    None => sleep(Duration::from_millis(5)).await,
                }
            }
        });
        *guard = Some(handle);
    }

    pub async fn submit(&self, request: DispatchRequest) -> Result<SubmitHandle, SoulError> {
        self.spawn().await;
        let hint = request.routing_hint.clone();
        let route = self.registry.route_resolve(hint).await?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tool_name = request.tool_call.tool.clone();
        let action_id = self
            .runtime
            .enqueue(route.mutex_key.clone(), request, route, tx);
        metrics::record_enqueued(&tool_name);
        Ok(SubmitHandle {
            action_id,
            receiver: rx,
        })
    }

    pub async fn cancel(&self, action: ActionId) -> Result<bool, SoulError> {
        match self.runtime.cancel(&action) {
            Some((request, route)) => {
                metrics::record_cancelled(&request.tool_call.tool);
                let task_id = request.tool_call.task_id.as_ref().map(|tid| tid.0.clone());
                log_cancelled(
                    &self.state_center,
                    &route,
                    &request.tool_call.tool,
                    &route.mutex_key,
                    &action,
                    task_id.as_deref(),
                    self.runtime.pending(),
                    self.runtime.global_slots().available_permits(),
                )
                .await;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub async fn cancel_call(&self, call_id: &str) -> Result<bool, SoulError> {
        self.spawn().await;
        match self.runtime.cancel_call(call_id) {
            Some((action_id, request, route)) => {
                metrics::record_cancelled(&request.tool_call.tool);
                let task_id = request.tool_call.task_id.as_ref().map(|tid| tid.0.clone());
                log_cancelled(
                    &self.state_center,
                    &route,
                    &request.tool_call.tool,
                    &route.mutex_key,
                    &action_id,
                    task_id.as_deref(),
                    self.runtime.pending(),
                    self.runtime.global_slots().available_permits(),
                )
                .await;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub async fn cancel_task(&self, task_id: &str) -> Result<usize, SoulError> {
        self.spawn().await;
        let cancelled = self.runtime.cancel_task(task_id);
        for (action_id, request, route) in &cancelled {
            metrics::record_cancelled(&request.tool_call.tool);
            let task_ref = request.tool_call.task_id.as_ref().map(|tid| tid.0.clone());
            log_cancelled(
                &self.state_center,
                route,
                &request.tool_call.tool,
                &route.mutex_key,
                action_id,
                task_ref.as_deref(),
                self.runtime.pending(),
                self.runtime.global_slots().available_permits(),
            )
            .await;
        }
        Ok(cancelled.len())
    }
}

async fn dispatch_job<R, E>(
    _registry: &Arc<R>,
    runtime: &Arc<SchedulerRuntime>,
    executor: &Arc<E>,
    state_center: &Arc<dyn StateCenter>,
    ready: ReadyJob,
) -> Result<(), SoulError>
where
    R: Registry + Send + Sync + 'static,
    E: ToolExecutor + Send + Sync + 'static,
{
    let request = ready.request().clone();
    let route = ready.route().clone();
    let mutex_key = ready.mutex_key();
    let mut completion = ready.take_completion();
    let action_id = ready.id();
    let task_id = ready.task_id();

    let max_retries = request.options.retry.max as u32;
    let backoff = request.options.retry.backoff;
    let timeout = request.options.timeout;
    let tool_name = request.tool_call.tool.clone();
    let mut attempt: u32 = 0;
    metrics::record_started(&tool_name);
    let job_timer = Instant::now();
    loop {
        let exec =
            tokio::time::timeout(timeout, executor.execute(request.clone(), route.clone())).await;
        let current_err = match exec {
            Ok(Ok(outcome)) => {
                let timeline = runtime.finish_job(ready);
                if let Some(tx) = completion.take() {
                    let _ = tx.send(DispatchOutput::ok(
                        route.clone(),
                        timeline.clone(),
                        outcome.output.clone(),
                    ));
                }
                metrics::record_completed(&tool_name, job_timer.elapsed());
                log_timeline(
                    state_center,
                    &route,
                    &tool_name,
                    &mutex_key,
                    &timeline,
                    runtime.pending(),
                    runtime.global_slots().available_permits(),
                    attempt,
                    &action_id,
                    task_id.as_deref(),
                    outcome.output.as_ref(),
                )
                .await;
                return Ok(());
            }
            Ok(Err(err)) => err,
            Err(_) => SoulError::new(format!("tool {} timed out after {:?}", tool_name, timeout)),
        };

        if attempt >= max_retries {
            let final_err = current_err;
            let timeline = runtime.finish_job(ready);
            if let Some(tx) = completion.take() {
                let _ = tx.send(DispatchOutput::err(
                    route.clone(),
                    final_err.clone(),
                    timeline.clone(),
                    None,
                ));
            }
            metrics::record_failed(&tool_name, job_timer.elapsed());
            log_failure(
                state_center,
                &route,
                &tool_name,
                &mutex_key,
                &timeline,
                runtime.pending(),
                runtime.global_slots().available_permits(),
                &final_err,
                attempt + 1,
                &action_id,
                task_id.as_deref(),
                None,
            )
            .await;
            return Err(final_err);
        }

        attempt += 1;
        let sleep_dur = backoff * attempt;
        sleep(sleep_dur).await;
    }
}

async fn log_timeline(
    state_center: &Arc<dyn StateCenter>,
    route: &ExecRoute,
    tool: &str,
    mutex_key: &str,
    timeline: &DispatchTimeline,
    pending: usize,
    slots_available: usize,
    attempts: u32,
    action_id: &ActionId,
    task_id: Option<&str>,
    output: Option<&Value>,
) {
    let (queue_wait_ms, run_ms) = compute_durations(timeline);
    info!(
        target: "scheduler",
        tool,
        mutex_key,
        attempts = attempts + 1,
        wait_ms = queue_wait_ms,
        run_ms,
        pending,
        slots_available,
        "tool execution completed"
    );
    let event = StateEvent::dispatch_success(DispatchEvent::success(
        action_id.clone(),
        task_id.map(|s| s.to_string()),
        route.clone(),
        tool.to_string(),
        mutex_key.to_string(),
        attempts + 1,
        queue_wait_ms,
        run_ms,
        pending,
        slots_available,
        output.cloned(),
    ));
    if let Err(err) = state_center.append(event).await {
        warn!("state center append failed: {err}");
    }
    record_success_metrics(queue_wait_ms, run_ms, attempts + 1);
}

async fn log_failure(
    state_center: &Arc<dyn StateCenter>,
    route: &ExecRoute,
    tool: &str,
    mutex_key: &str,
    timeline: &DispatchTimeline,
    pending: usize,
    slots_available: usize,
    error: &SoulError,
    attempts: u32,
    action_id: &ActionId,
    task_id: Option<&str>,
    output: Option<&Value>,
) {
    let (queue_wait_ms, run_ms) = compute_durations(timeline);
    warn!(
        target: "scheduler",
        tool,
        mutex_key,
        attempts,
        wait_ms = queue_wait_ms,
        run_ms,
        pending,
        slots_available,
        error = %error,
        "tool execution failed"
    );
    let event = StateEvent::dispatch_failure(DispatchEvent::failure(
        action_id.clone(),
        task_id.map(|s| s.to_string()),
        route.clone(),
        tool.to_string(),
        mutex_key.to_string(),
        attempts,
        queue_wait_ms,
        run_ms,
        pending,
        slots_available,
        error.clone(),
        output.cloned(),
    ));
    if let Err(err) = state_center.append(event).await {
        warn!("state center append failed: {err}");
    }
    record_failure_metrics(queue_wait_ms, run_ms, attempts);
}

async fn log_cancelled(
    state_center: &Arc<dyn StateCenter>,
    route: &ExecRoute,
    tool: &str,
    mutex_key: &str,
    action_id: &ActionId,
    task_id: Option<&str>,
    pending: usize,
    slots_available: usize,
) {
    warn!(
        target: "scheduler",
        tool,
        mutex_key,
        pending,
        slots_available,
        "tool execution cancelled"
    );
    let event = StateEvent::dispatch_failure(DispatchEvent::failure(
        action_id.clone(),
        task_id.map(|s| s.to_string()),
        route.clone(),
        tool.to_string(),
        mutex_key.to_string(),
        0,
        0,
        0,
        pending,
        slots_available,
        SoulError::new("cancelled"),
        None,
    ));
    if let Err(err) = state_center.append(event).await {
        warn!("state center append failed: {err}");
    }
}

fn compute_durations(timeline: &DispatchTimeline) -> (u64, u64) {
    let wait_ms = timeline
        .started_at
        .map(|start| start.duration_since(timeline.enqueued_at).as_millis() as u64)
        .unwrap_or(0);
    let run_ms = match (timeline.started_at, timeline.finished_at) {
        (Some(start), Some(finish)) => finish.duration_since(start).as_millis() as u64,
        _ => 0,
    };
    (wait_ms, run_ms)
}

fn record_success_metrics(_wait_ms: u64, _run_ms: u64, _attempts: u32) {}

fn record_failure_metrics(_wait_ms: u64, _run_ms: u64, _attempts: u32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{ToolDispatchResult, ToolExecutor};
    use crate::model::{CallOptions, DispatchRequest, Priority, SchedulerConfig};
    use soulbrowser_core_types::{ExecRoute, FrameId, PageId, RoutingHint, SessionId, ToolCall};
    use soulbrowser_registry::SessionCtx;
    use soulbrowser_state_center::{InMemoryStateCenter, NoopStateCenter, StateCenter, StateEvent};
    use tokio::sync::Mutex as AsyncMutex;

    #[derive(Clone)]
    struct MockRegistry {
        route: ExecRoute,
        calls: Arc<AsyncMutex<Vec<Option<RoutingHint>>>>,
    }

    #[async_trait::async_trait]
    impl Registry for MockRegistry {
        async fn session_create(&self, _profile: &str) -> Result<SessionId, SoulError> {
            Err(SoulError::new("unimplemented"))
        }

        async fn page_open(&self, _session: SessionId) -> Result<PageId, SoulError> {
            Err(SoulError::new("unimplemented"))
        }

        async fn page_close(&self, _page: PageId) -> Result<(), SoulError> {
            Err(SoulError::new("unimplemented"))
        }

        async fn page_focus(&self, _page: PageId) -> Result<(), SoulError> {
            Err(SoulError::new("unimplemented"))
        }

        async fn frame_focus(&self, _page: PageId, _frame: FrameId) -> Result<(), SoulError> {
            Err(SoulError::new("unimplemented"))
        }

        async fn route_resolve(&self, hint: Option<RoutingHint>) -> Result<ExecRoute, SoulError> {
            self.calls.lock().await.push(hint);
            Ok(self.route.clone())
        }

        async fn session_list(&self) -> Vec<SessionCtx> {
            Vec::new()
        }
    }

    struct MockExecutor {
        executions: Arc<AsyncMutex<usize>>,
    }

    #[async_trait::async_trait]
    impl ToolExecutor for MockExecutor {
        async fn execute(
            &self,
            _request: DispatchRequest,
            _route: ExecRoute,
        ) -> Result<ToolDispatchResult, SoulError> {
            let mut guard = self.executions.lock().await;
            *guard += 1;
            Ok(ToolDispatchResult { output: None })
        }
    }

    struct FailingExecutor;

    #[async_trait::async_trait]
    impl ToolExecutor for FailingExecutor {
        async fn execute(
            &self,
            _request: DispatchRequest,
            _route: ExecRoute,
        ) -> Result<ToolDispatchResult, SoulError> {
            Err(SoulError::new("executor failure"))
        }
    }

    fn mock_route() -> ExecRoute {
        ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new())
    }

    fn mock_request() -> DispatchRequest {
        DispatchRequest {
            tool_call: ToolCall {
                tool: "click".into(),
                ..Default::default()
            },
            options: CallOptions {
                priority: Priority::Standard,
                ..CallOptions::default()
            },
            routing_hint: Some(RoutingHint::default()),
        }
    }

    #[tokio::test]
    async fn dispatch_records_state_center_events() {
        let registry = Arc::new(MockRegistry {
            route: mock_route(),
            calls: Arc::new(AsyncMutex::new(Vec::new())),
        });
        let runtime = Arc::new(SchedulerRuntime::new(SchedulerConfig::default()));
        let executor = Arc::new(MockExecutor {
            executions: Arc::new(AsyncMutex::new(0)),
        });
        let state_center = Arc::new(InMemoryStateCenter::new(16));
        let state_center_dyn: Arc<dyn StateCenter> = state_center.clone();
        let orchestrator = Orchestrator::new(
            registry.clone(),
            runtime.clone(),
            executor.clone(),
            state_center_dyn,
        );

        let handle = orchestrator.submit(mock_request()).await.unwrap();
        orchestrator.spawn().await;
        handle.receiver.await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let events = state_center.snapshot();
        assert!(!events.is_empty(), "expected at least one dispatch event");
        match events.last() {
            Some(StateEvent::Dispatch(event)) => {
                assert_eq!(event.tool, "click");
                assert!(matches!(
                    event.status,
                    soulbrowser_state_center::DispatchStatus::Success
                ));
            }
            _ => panic!("unexpected event type"),
        }
    }

    #[tokio::test]
    async fn dispatch_failure_records_error_event() {
        let registry = Arc::new(MockRegistry {
            route: mock_route(),
            calls: Arc::new(AsyncMutex::new(Vec::new())),
        });
        let runtime = Arc::new(SchedulerRuntime::new(SchedulerConfig::default()));
        let executor = Arc::new(FailingExecutor);
        let state_center = Arc::new(InMemoryStateCenter::new(8));
        let state_center_dyn: Arc<dyn StateCenter> = state_center.clone();
        let orchestrator = Orchestrator::new(
            registry.clone(),
            runtime.clone(),
            executor,
            state_center_dyn,
        );

        let handle = orchestrator.submit(mock_request()).await.unwrap();
        orchestrator.spawn().await;
        let output = handle.receiver.await.unwrap();
        assert!(output.error.is_some());

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let events = state_center.snapshot();
        let failure = events
            .into_iter()
            .find_map(|event| match event {
                StateEvent::Dispatch(dispatch)
                    if matches!(
                        dispatch.status,
                        soulbrowser_state_center::DispatchStatus::Failure
                    ) =>
                {
                    Some(dispatch)
                }
                _ => None,
            })
            .expect("expected failure event");
        assert_eq!(failure.tool, "click");
        assert!(failure.error.is_some());
    }

    #[tokio::test]
    async fn cancel_records_event() {
        let registry = Arc::new(MockRegistry {
            route: mock_route(),
            calls: Arc::new(AsyncMutex::new(Vec::new())),
        });
        let runtime = Arc::new(SchedulerRuntime::new(SchedulerConfig::default()));
        let executor = Arc::new(MockExecutor {
            executions: Arc::new(AsyncMutex::new(0)),
        });
        let state_center = Arc::new(InMemoryStateCenter::new(8));
        let state_center_dyn: Arc<dyn StateCenter> = state_center.clone();
        let orchestrator = Orchestrator::new(
            registry.clone(),
            runtime.clone(),
            executor,
            state_center_dyn,
        );

        let handle = orchestrator.submit(mock_request()).await.unwrap();
        let cancelled = orchestrator.cancel(handle.action_id.clone()).await.unwrap();
        assert!(cancelled);

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let events = state_center.snapshot();
        assert!(events.iter().any(|event| matches!(event, StateEvent::Dispatch(dispatch) if dispatch.error.as_ref().map(|e| e.to_string()) == Some("cancelled".to_string()))));
    }

    #[tokio::test]
    async fn submit_enqueues_and_worker_drains() {
        let registry = Arc::new(MockRegistry {
            route: mock_route(),
            calls: Arc::new(AsyncMutex::new(Vec::new())),
        });
        let runtime = Arc::new(SchedulerRuntime::new(SchedulerConfig::default()));
        let executor = Arc::new(MockExecutor {
            executions: Arc::new(AsyncMutex::new(0)),
        });
        let state_center: Arc<dyn StateCenter> = NoopStateCenter::new();
        let orchestrator = Orchestrator::new(
            registry.clone(),
            runtime.clone(),
            executor.clone(),
            state_center,
        );

        let handle = orchestrator.submit(mock_request()).await.unwrap();
        assert_eq!(runtime.pending(), 1);
        orchestrator.spawn().await;

        let output = handle.receiver.await.unwrap();
        assert_eq!(runtime.pending(), 0);

        let calls = registry.calls.lock().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(output.route.session, registry.route.session);
        assert!(output.timeline.started_at.is_some());
        assert!(output.timeline.finished_at.is_some());

        let executions = executor.executions.lock().await;
        assert_eq!(*executions, 1);
    }
}
