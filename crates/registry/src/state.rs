use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use dashmap::DashMap;
use network_tap_light::NetworkSnapshot;
use parking_lot::RwLock;
use soulbrowser_policy_center::{default_snapshot, PolicyView};
use soulbrowser_state_center::{RegistryAction, RegistryEvent, StateCenter, StateEvent};
use tracing::warn;

use soulbrowser_core_types::{
    ExecRoute, FrameId, PageId, RoutePrefer, RoutingHint, SessionId, SoulError,
};

use crate::{
    api::Registry,
    errors::RegistryError,
    metrics,
    model::{FrameCtx, LifeState, PageCtx, SessionCtx},
};

fn now() -> Instant {
    Instant::now()
}

/// In-memory registry implementation used during Phase 1 bring-up.
pub struct RegistryImpl {
    pub sessions: DashMap<SessionId, Arc<RwLock<SessionCtx>>>,
    pub pages: DashMap<PageId, Arc<RwLock<PageCtx>>>,
    pub frames: DashMap<FrameId, Arc<RwLock<FrameCtx>>>,
    state_center: Option<Arc<dyn StateCenter + Send + Sync>>,
    policy_view: Arc<RwLock<PolicyView>>,
}

impl RegistryImpl {
    pub fn new() -> Self {
        let snapshot = default_snapshot();
        let view = Arc::new(RwLock::new(PolicyView::from(snapshot)));
        Self {
            sessions: DashMap::new(),
            pages: DashMap::new(),
            frames: DashMap::new(),
            state_center: None,
            policy_view: view,
        }
    }

    pub fn with_state_center(
        state_center: Arc<dyn StateCenter + Send + Sync>,
        policy_view: Arc<RwLock<PolicyView>>,
    ) -> Self {
        Self {
            sessions: DashMap::new(),
            pages: DashMap::new(),
            frames: DashMap::new(),
            state_center: Some(state_center),
            policy_view,
        }
    }

    pub fn update_policy(&self, view: PolicyView) {
        *self.policy_view.write() = view;
    }

    pub fn health_probe_tick(&self) {
        self.emit_registry_event(RegistryAction::HealthProbeTick, None, None, None, None);
    }

    pub fn apply_network_snapshot(
        &self,
        page: &PageId,
        snapshot: &NetworkSnapshot,
    ) -> Result<(), SoulError> {
        let page_ctx = self.ensure_page(page)?;
        let session = {
            let mut guard = page_ctx.write();
            guard.health.update_from_snapshot(snapshot);
            if !snapshot.quiet {
                guard.last_active_at = Instant::now();
            }
            guard.session.clone()
        };
        metrics::record_page_health_update(snapshot);
        self.emit_registry_event(
            RegistryAction::PageHealthUpdated,
            Some(session),
            Some(page.clone()),
            None,
            None,
        );
        Ok(())
    }

    pub fn update_page_url(&self, page: &PageId, url: String) {
        if let Ok(page_ctx) = self.ensure_page(page) {
            let mut guard = page_ctx.write();
            guard.url = Some(url);
            guard.last_active_at = now();
        }
    }

    fn emit_registry_event(
        &self,
        action: RegistryAction,
        session: Option<SessionId>,
        page: Option<PageId>,
        frame: Option<FrameId>,
        note: Option<String>,
    ) {
        if let Some(center) = &self.state_center {
            let event = RegistryEvent::new(action, session, page, frame, note);
            let center = Arc::clone(center);
            tokio::spawn(async move {
                if let Err(err) = center.append(StateEvent::registry(event)).await {
                    warn!("registry state center append failed: {err}");
                }
            });
        }
    }

    pub(crate) fn ensure_session(
        &self,
        session: &SessionId,
    ) -> Result<Arc<RwLock<SessionCtx>>, SoulError> {
        self.sessions
            .get(session)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| {
                RegistryError::NotFound.into_soul_error(format!("session {}", session.0))
            })
    }

    pub(crate) fn ensure_page(&self, page: &PageId) -> Result<Arc<RwLock<PageCtx>>, SoulError> {
        self.pages
            .get(page)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RegistryError::NotFound.into_soul_error(format!("page {}", page.0)))
    }

    pub(crate) fn ensure_frame(&self, frame: &FrameId) -> Result<Arc<RwLock<FrameCtx>>, SoulError> {
        self.frames
            .get(frame)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RegistryError::NotFound.into_soul_error(format!("frame {}", frame.0)))
    }

    fn route_for_frame(&self, frame_id: &FrameId) -> Result<ExecRoute, SoulError> {
        let frame = self.ensure_frame(frame_id)?;
        let frame = frame.read().clone();
        let page = self.ensure_page(&frame.page)?;
        let page = page.read().clone();
        self.build_exec_route(page.session, page.id, frame.id)
    }

    fn route_for_page(
        &self,
        page_id: &PageId,
        prefer: Option<RoutePrefer>,
    ) -> Result<ExecRoute, SoulError> {
        let page = self.ensure_page(page_id)?;
        let page = page.read().clone();
        let frame = self.choose_frame(&page, prefer).ok_or_else(|| {
            RegistryError::NotFound.into_soul_error(format!("frame for page {}", page_id.0))
        })?;
        self.build_exec_route(page.session, page.id, frame)
    }

    fn route_for_session(
        &self,
        session_id: &SessionId,
        prefer: Option<RoutePrefer>,
    ) -> Result<ExecRoute, SoulError> {
        let session = self.ensure_session(session_id)?;
        let session = session.read().clone();
        if let Some(page_id) = session.focused_page.clone() {
            return self.route_for_page(&page_id, prefer);
        }
        let candidate = self
            .pages
            .iter()
            .find(|entry| entry.value().read().session == session.id)
            .map(|entry| entry.key().clone());
        if let Some(pid) = candidate {
            return self.route_for_page(&pid, prefer);
        }
        Err(RegistryError::NotFound
            .into_soul_error(format!("no pages for session {}", session_id.0)))
    }

    fn route_default(&self) -> Result<ExecRoute, SoulError> {
        if let Some(entry) = self.sessions.iter().next() {
            let session = entry.value().read().clone();
            return self.route_for_session(&session.id, None);
        }
        Err(RegistryError::NotFound.into_soul_error("no sessions available"))
    }

    fn pick_recent_page(&self, session: &SessionId, exclude: Option<&PageId>) -> Option<PageId> {
        let mut selected: Option<(PageId, Instant)> = None;
        for entry in self.pages.iter() {
            let page_id = entry.key().clone();
            if exclude.map(|ex| ex == &page_id).unwrap_or(false) {
                continue;
            }
            let ctx = entry.value().read();
            if ctx.session != *session {
                continue;
            }
            match &mut selected {
                Some((_, ts)) if ctx.last_active_at <= *ts => {}
                _ => selected = Some((page_id, ctx.last_active_at)),
            }
        }
        selected.map(|(id, _)| id)
    }

    fn build_exec_route(
        &self,
        session: SessionId,
        page: PageId,
        frame: FrameId,
    ) -> Result<ExecRoute, SoulError> {
        if !self.sessions.contains_key(&session)
            || !self.pages.contains_key(&page)
            || !self.frames.contains_key(&frame)
        {
            return Err(RegistryError::NotFound.into_soul_error("route components missing"));
        }
        Ok(ExecRoute::new(session, page, frame))
    }

    fn choose_frame(&self, page: &PageCtx, prefer: Option<RoutePrefer>) -> Option<FrameId> {
        match prefer {
            Some(RoutePrefer::MainFrame) => page
                .main_frame
                .clone()
                .or_else(|| page.focused_frame.clone()),
            Some(RoutePrefer::Focused) => page
                .focused_frame
                .clone()
                .or_else(|| page.main_frame.clone()),
            Some(RoutePrefer::RecentNav) => page
                .focused_frame
                .clone()
                .or_else(|| page.main_frame.clone()),
            None => page
                .focused_frame
                .clone()
                .or_else(|| page.main_frame.clone()),
        }
    }

    pub fn frame_attached(
        &self,
        page: &PageId,
        parent: Option<FrameId>,
        is_main: bool,
    ) -> Result<FrameId, SoulError> {
        let page_arc = self.ensure_page(page)?;
        let mut page_ctx = page_arc.write();
        let parent_id = if let Some(ref pid) = parent {
            let parent_arc = self.ensure_frame(pid)?;
            let parent_ctx = parent_arc.read().clone();
            if parent_ctx.page != *page {
                return Err(
                    RegistryError::OwnershipConflict.into_soul_error("parent frame not in page")
                );
            }
            Some(parent_ctx.id.clone())
        } else {
            None
        };

        let (frame_id, mut frame_ctx) = FrameCtx::new(page.clone(), parent_id.clone(), is_main);
        frame_ctx.state = LifeState::Ready;

        if let Some(parent_id) = parent_id {
            if let Some(parent_arc) = self.frames.get(&parent_id) {
                let mut guard = parent_arc.value().write();
                guard.children.push(frame_id.clone());
            }
        }

        if is_main {
            page_ctx.main_frame = Some(frame_id.clone());
        }

        self.frames
            .insert(frame_id.clone(), Arc::new(RwLock::new(frame_ctx)));
        metrics::set_frame_count(self.frames.len());

        if page_ctx.focused_frame.is_none() {
            page_ctx.focused_frame = page_ctx.main_frame.clone().or(Some(frame_id.clone()));
        }

        page_ctx.last_active_at = now();
        let session_for_event = page_ctx.session.clone();
        let page_for_event = page.clone();
        let frame_for_event = frame_id.clone();
        drop(page_ctx);

        self.emit_registry_event(
            RegistryAction::FrameAttached,
            Some(session_for_event),
            Some(page_for_event),
            Some(frame_for_event.clone()),
            None,
        );

        Ok(frame_for_event)
    }

    pub fn frame_detached(&self, frame: &FrameId) -> Result<(), SoulError> {
        let frame_arc = self.ensure_frame(frame)?;
        let frame_ctx = frame_arc.read().clone();
        let page_id = frame_ctx.page.clone();

        self.remove_frame_recursive(frame);
        metrics::set_frame_count(self.frames.len());

        let page_arc = self.ensure_page(&page_id)?;
        let mut page_ctx = page_arc.write();
        let session_id = page_ctx.session.clone();
        if page_ctx.main_frame.as_ref() == Some(frame) {
            page_ctx.main_frame = None;
        }
        if page_ctx.focused_frame.as_ref() == Some(frame) {
            page_ctx.focused_frame = page_ctx.main_frame.clone().or_else(|| {
                let mut fallback = None;
                for entry in self.frames.iter() {
                    let ctx = entry.value().read();
                    if ctx.page == page_id {
                        fallback = Some(ctx.id.clone());
                        break;
                    }
                }
                fallback
            });
        }
        page_ctx.last_active_at = now();
        drop(page_ctx);

        self.emit_registry_event(
            RegistryAction::FrameDetached,
            Some(session_id),
            Some(page_id.clone()),
            Some(frame.clone()),
            None,
        );
        Ok(())
    }

    fn remove_frame_recursive(&self, frame: &FrameId) {
        if let Some(frame_arc) = self.frames.remove(frame).map(|(_, arc)| arc) {
            let frame_ctx = frame_arc.read().clone();
            if let Some(parent_id) = frame_ctx.parent.clone() {
                if let Some(parent_arc) = self.frames.get(&parent_id) {
                    let mut parent = parent_arc.value().write();
                    parent.children.retain(|child| child != frame);
                }
            }
            for child in frame_ctx.children {
                self.remove_frame_recursive(&child);
            }
        }
    }
}

#[async_trait]
impl Registry for RegistryImpl {
    async fn session_create(&self, profile: &str) -> Result<SessionId, SoulError> {
        let (id, ctx) = SessionCtx::new(profile);
        self.sessions.insert(id.clone(), Arc::new(RwLock::new(ctx)));
        metrics::set_session_count(self.sessions.len());
        self.emit_registry_event(
            RegistryAction::SessionCreated,
            Some(id.clone()),
            None,
            None,
            Some(profile.to_string()),
        );
        Ok(id)
    }

    async fn page_open(&self, session: SessionId) -> Result<PageId, SoulError> {
        self.ensure_session(&session)?;
        let allow_multiple_pages = self.policy_view.read().registry.allow_multiple_pages;
        if !allow_multiple_pages {
            let has_page = self
                .pages
                .iter()
                .any(|entry| entry.value().read().session == session);
            if has_page {
                return Err(
                    RegistryError::LimitReached.into_soul_error("session already has a page")
                );
            }
        }
        let (page_id, mut page_ctx) = PageCtx::new(session.clone());
        let (frame_id, mut frame_ctx) = FrameCtx::new(page_id.clone(), None, true);
        page_ctx.main_frame = Some(frame_id.clone());
        page_ctx.focused_frame = Some(frame_id.clone());
        page_ctx.state = LifeState::Ready;
        frame_ctx.state = LifeState::Ready;

        self.frames
            .insert(frame_id.clone(), Arc::new(RwLock::new(frame_ctx)));
        self.pages
            .insert(page_id.clone(), Arc::new(RwLock::new(page_ctx)));
        metrics::set_page_count(self.pages.len());
        metrics::set_frame_count(self.frames.len());

        if let Some(session_entry) = self.sessions.get(&session) {
            let mut session = session_entry.value().write();
            if session.focused_page.is_none() {
                session.focused_page = Some(page_id.clone());
            }
        }

        self.emit_registry_event(
            RegistryAction::PageOpened,
            Some(session),
            Some(page_id.clone()),
            Some(frame_id),
            None,
        );
        Ok(page_id)
    }

    async fn page_close(&self, page: PageId) -> Result<(), SoulError> {
        let (session_id, _) = {
            let page_arc = self.ensure_page(&page)?;
            let mut page_ctx = page_arc.write();
            page_ctx.state = LifeState::Closing;
            let session = page_ctx.session.clone();
            (session, page_ctx.focused_frame.clone())
        };

        let frames: Vec<FrameId> = self
            .frames
            .iter()
            .filter(|entry| entry.value().read().page == page)
            .map(|entry| entry.key().clone())
            .collect();
        for frame in frames {
            self.frames.remove(&frame);
        }

        self.pages.remove(&page);

        if let Some(session_entry) = self.sessions.get(&session_id) {
            let mut session_ctx = session_entry.value().write();
            if session_ctx.focused_page.as_ref() == Some(&page) {
                session_ctx.focused_page = self.pick_recent_page(&session_id, Some(&page));
            }
            if session_ctx.focused_page.is_none() {
                session_ctx.state = LifeState::Ready;
            }
        }

        self.emit_registry_event(
            RegistryAction::PageClosed,
            Some(session_id),
            Some(page.clone()),
            None,
            None,
        );
        metrics::set_page_count(self.pages.len());
        metrics::set_frame_count(self.frames.len());
        Ok(())
    }

    async fn page_focus(&self, page: PageId) -> Result<(), SoulError> {
        let session_id = {
            let page_arc = self.ensure_page(&page)?;
            let mut page_ctx = page_arc.write();
            page_ctx.state = LifeState::Active;
            page_ctx.last_active_at = now();
            if page_ctx.focused_frame.is_none() {
                page_ctx.focused_frame = page_ctx.main_frame.clone();
            }
            page_ctx.session.clone()
        };

        {
            let session_arc = self.ensure_session(&session_id)?;
            let mut session = session_arc.write();
            session.focused_page = Some(page.clone());
            session.state = LifeState::Active;
        }
        self.emit_registry_event(
            RegistryAction::PageFocused,
            Some(session_id),
            Some(page),
            None,
            None,
        );
        Ok(())
    }

    async fn frame_focus(&self, page: PageId, frame: FrameId) -> Result<(), SoulError> {
        let frame_arc = self.ensure_frame(&frame)?;
        let frame_ctx = frame_arc.read().clone();
        if frame_ctx.page != page {
            return Err(
                RegistryError::OwnershipConflict.into_soul_error("frame does not belong to page")
            );
        }

        let session_id = {
            let page_arc = self.ensure_page(&page)?;
            let mut page_ctx = page_arc.write();
            page_ctx.focused_frame = Some(frame.clone());
            page_ctx.state = LifeState::Active;
            page_ctx.last_active_at = now();
            page_ctx.session.clone()
        };

        {
            let session_arc = self.ensure_session(&session_id)?;
            let mut session = session_arc.write();
            session.focused_page = Some(page.clone());
            session.state = LifeState::Active;
        }

        self.emit_registry_event(
            RegistryAction::FrameFocused,
            Some(session_id),
            Some(page),
            Some(frame),
            None,
        );
        Ok(())
    }

    async fn route_resolve(&self, hint: Option<RoutingHint>) -> Result<ExecRoute, SoulError> {
        if let Some(hint) = hint {
            if let Some(frame_id) = hint.frame {
                return self.route_for_frame(&frame_id);
            }
            if let Some(page_id) = hint.page {
                return self.route_for_page(&page_id, hint.prefer);
            }
            if let Some(session_id) = hint.session {
                return self.route_for_session(&session_id, hint.prefer);
            }
        }
        self.route_default()
    }

    async fn session_list(&self) -> Vec<SessionCtx> {
        self.sessions
            .iter()
            .map(|entry| entry.value().read().clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soulbrowser_core_types::RoutingHint;
    use std::sync::Arc;

    #[tokio::test]
    async fn creates_and_lists_sessions() {
        let registry = RegistryImpl::new();

        let id = registry.session_create("default").await.unwrap();
        let sessions = registry.session_list().await;

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, id);
        assert_eq!(sessions[0].profile_name, "default");
    }

    #[tokio::test]
    async fn route_defaults_to_focused_page() {
        let registry = RegistryImpl::new();
        let session = registry.session_create("user").await.unwrap();
        let _page_a = registry.page_open(session.clone()).await.unwrap();
        let page_b = registry.page_open(session.clone()).await.unwrap();

        registry.page_focus(page_b.clone()).await.unwrap();

        let exec = registry.route_resolve(None).await.unwrap();
        assert_eq!(exec.session, session);
        assert_eq!(exec.page, page_b);
    }

    #[tokio::test]
    async fn frame_focus_updates_route() {
        let registry = RegistryImpl::new();
        let session = registry.session_create("user").await.unwrap();
        let page = registry.page_open(session.clone()).await.unwrap();

        // simulate adding a secondary frame
        let frame_id = {
            let page_ctx = registry.ensure_page(&page).unwrap();
            let page_ctx_read = page_ctx.read().clone();
            let (frame_id, mut frame_ctx) = FrameCtx::new(
                page_ctx_read.id.clone(),
                page_ctx_read.main_frame.clone(),
                false,
            );
            frame_ctx.state = LifeState::Ready;
            registry
                .frames
                .insert(frame_id.clone(), Arc::new(RwLock::new(frame_ctx)));
            frame_id
        };

        registry
            .frame_focus(page.clone(), frame_id.clone())
            .await
            .unwrap();
        let exec = registry
            .route_resolve(Some(RoutingHint {
                page: Some(page.clone()),
                ..Default::default()
            }))
            .await
            .unwrap();

        assert_eq!(exec.frame, frame_id);
    }

    #[tokio::test]
    async fn frame_attach_records_parent_child() {
        let registry = RegistryImpl::new();
        let session = registry.session_create("user").await.unwrap();
        let page = registry.page_open(session.clone()).await.unwrap();

        let main_frame = {
            let page_ctx = registry.ensure_page(&page).unwrap();
            let main = page_ctx.read().main_frame.clone().unwrap();
            main
        };

        let child = registry
            .frame_attached(&page, Some(main_frame.clone()), false)
            .unwrap();

        let main_ctx = registry.ensure_frame(&main_frame).unwrap();
        assert_eq!(main_ctx.read().children, vec![child.clone()]);

        registry.frame_detached(&child).unwrap();

        let main_ctx = registry.ensure_frame(&main_frame).unwrap();
        assert!(main_ctx.read().children.is_empty());
        assert!(registry.frames.get(&child).is_none());
    }

    #[tokio::test]
    async fn page_close_reassigns_focus() {
        let registry = RegistryImpl::new();
        let session = registry.session_create("user").await.unwrap();
        let page_a = registry.page_open(session.clone()).await.unwrap();
        let page_b = registry.page_open(session.clone()).await.unwrap();

        registry.page_focus(page_b.clone()).await.unwrap();
        registry.page_close(page_b.clone()).await.unwrap();

        let exec = registry.route_resolve(None).await.unwrap();
        assert_eq!(exec.page, page_a);
        assert!(registry.pages.get(&page_b).is_none());
    }

    #[tokio::test]
    async fn frame_detach_falls_back_to_remaining() {
        let registry = RegistryImpl::new();
        let session = registry.session_create("user").await.unwrap();
        let page = registry.page_open(session.clone()).await.unwrap();

        let main_frame = {
            let page_ctx = registry.ensure_page(&page).unwrap();
            let main = page_ctx.read().main_frame.clone().unwrap();
            main
        };

        let child = registry
            .frame_attached(&page, Some(main_frame.clone()), false)
            .unwrap();
        registry
            .frame_focus(page.clone(), child.clone())
            .await
            .unwrap();

        registry.frame_detached(&child).unwrap();

        let exec = registry
            .route_resolve(Some(RoutingHint {
                page: Some(page.clone()),
                ..Default::default()
            }))
            .await
            .unwrap();

        assert_eq!(exec.frame, main_frame);
    }

    #[tokio::test]
    async fn closing_last_page_returns_error_on_route() {
        let registry = RegistryImpl::new();
        let session = registry.session_create("user").await.unwrap();
        let page = registry.page_open(session.clone()).await.unwrap();

        registry.page_close(page.clone()).await.unwrap();

        let err = registry.route_resolve(None).await.err().unwrap();
        let msg = err.to_string();
        assert!(msg.contains("no sessions") || msg.contains("no pages"));
    }
}
