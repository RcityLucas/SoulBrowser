use crate::executor::ToolExecutor;
use crate::model::{DispatchRequest, SubmitHandle};
use crate::orchestrator::Orchestrator;
use crate::runtime::SchedulerRuntime;
use async_trait::async_trait;
use soulbrowser_core_types::{ActionId, SoulError};
use soulbrowser_registry::Registry;
use soulbrowser_state_center::StateCenter;
use std::sync::Arc;

#[async_trait]
pub trait Dispatcher: Send + Sync {
    async fn submit(&self, call: DispatchRequest) -> Result<SubmitHandle, SoulError>;
    async fn cancel(&self, action: ActionId) -> Result<bool, SoulError>;
    async fn cancel_call(&self, call_id: &str) -> Result<bool, SoulError>;
    async fn cancel_task(&self, task_id: &str) -> Result<usize, SoulError>;
}

pub struct SchedulerService<R, E>
where
    R: Registry + Send + Sync + 'static,
    E: ToolExecutor + Send + Sync + 'static,
{
    orchestrator: Orchestrator<R, E>,
}

impl<R, E> SchedulerService<R, E>
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
        let orchestrator = Orchestrator::new(registry, runtime, executor, state_center);
        Self { orchestrator }
    }

    pub async fn start(&self) {
        self.orchestrator.spawn().await;
    }
}

impl<R> SchedulerService<R, crate::executor::NoopExecutor>
where
    R: Registry + Send + Sync + 'static,
{
    pub fn with_noop_executor(
        registry: Arc<R>,
        runtime: Arc<SchedulerRuntime>,
        state_center: Arc<dyn StateCenter>,
    ) -> Self {
        let executor = Arc::new(crate::executor::NoopExecutor::default());
        Self::new(registry, runtime, executor, state_center)
    }
}

#[async_trait]
impl<R, E> Dispatcher for SchedulerService<R, E>
where
    R: Registry + Send + Sync + 'static,
    E: ToolExecutor + Send + Sync + 'static,
{
    async fn submit(&self, call: DispatchRequest) -> Result<SubmitHandle, SoulError> {
        self.orchestrator.submit(call).await
    }

    async fn cancel(&self, action: ActionId) -> Result<bool, SoulError> {
        self.orchestrator.cancel(action).await
    }

    async fn cancel_call(&self, call_id: &str) -> Result<bool, SoulError> {
        self.orchestrator.cancel_call(call_id).await
    }

    async fn cancel_task(&self, task_id: &str) -> Result<usize, SoulError> {
        self.orchestrator.cancel_task(task_id).await
    }
}

#[async_trait]
impl<D> Dispatcher for Arc<D>
where
    D: Dispatcher + ?Sized,
{
    async fn submit(&self, call: DispatchRequest) -> Result<SubmitHandle, SoulError> {
        (**self).submit(call).await
    }

    async fn cancel(&self, action: ActionId) -> Result<bool, SoulError> {
        (**self).cancel(action).await
    }

    async fn cancel_call(&self, call_id: &str) -> Result<bool, SoulError> {
        (**self).cancel_call(call_id).await
    }

    async fn cancel_task(&self, task_id: &str) -> Result<usize, SoulError> {
        (**self).cancel_task(task_id).await
    }
}
