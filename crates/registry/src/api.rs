#![allow(dead_code)]

use async_trait::async_trait;
use soulbrowser_core_types::{ExecRoute, FrameId, PageId, RoutingHint, SessionId, SoulError};

use crate::model::SessionCtx;

#[async_trait]
pub trait Registry: Send + Sync {
    async fn session_create(&self, profile: &str) -> Result<SessionId, SoulError>;
    async fn page_open(&self, session: SessionId) -> Result<PageId, SoulError>;
    async fn page_close(&self, page: PageId) -> Result<(), SoulError>;
    async fn page_focus(&self, page: PageId) -> Result<(), SoulError>;
    async fn frame_focus(&self, page: PageId, frame: FrameId) -> Result<(), SoulError>;
    async fn route_resolve(&self, hint: Option<RoutingHint>) -> Result<ExecRoute, SoulError>;
    async fn session_list(&self) -> Vec<SessionCtx>;
}

pub struct RegistryStub;

#[async_trait]
impl Registry for RegistryStub {
    async fn session_create(&self, _profile: &str) -> Result<SessionId, SoulError> {
        Err(SoulError::new("registry not implemented"))
    }

    async fn page_open(&self, _session: SessionId) -> Result<PageId, SoulError> {
        Err(SoulError::new("registry not implemented"))
    }

    async fn page_close(&self, _page: PageId) -> Result<(), SoulError> {
        Err(SoulError::new("registry not implemented"))
    }

    async fn page_focus(&self, _page: PageId) -> Result<(), SoulError> {
        Err(SoulError::new("registry not implemented"))
    }

    async fn frame_focus(&self, _page: PageId, _frame: FrameId) -> Result<(), SoulError> {
        Err(SoulError::new("registry not implemented"))
    }

    async fn route_resolve(&self, _hint: Option<RoutingHint>) -> Result<ExecRoute, SoulError> {
        Err(SoulError::new("registry not implemented"))
    }

    async fn session_list(&self) -> Vec<SessionCtx> {
        Vec::new()
    }
}
