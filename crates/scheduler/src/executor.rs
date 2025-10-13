use async_trait::async_trait;

use crate::model::DispatchRequest;
use soulbrowser_core_types::{ExecRoute, SoulError};

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, request: DispatchRequest, route: ExecRoute) -> Result<(), SoulError>;
}

#[derive(Clone, Copy, Default, Debug)]
pub struct NoopExecutor;

#[async_trait]
impl ToolExecutor for NoopExecutor {
    async fn execute(&self, _request: DispatchRequest, _route: ExecRoute) -> Result<(), SoulError> {
        Ok(())
    }
}
