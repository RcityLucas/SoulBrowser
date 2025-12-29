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
