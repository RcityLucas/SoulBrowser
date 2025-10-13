use std::time::Instant;

use crate::health::PageHealth;
use soulbrowser_core_types::{FrameId, PageId, SessionId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifeState {
    Init,
    Ready,
    Active,
    Closing,
    Closed,
    Lost,
}

#[derive(Clone, Debug)]
pub struct SessionCtx {
    pub id: SessionId,
    pub profile_name: String,
    pub created_at: Instant,
    pub focused_page: Option<PageId>,
    pub state: LifeState,
}

impl SessionCtx {
    pub fn new(profile_name: impl Into<String>) -> (SessionId, Self) {
        let id = SessionId::new();
        let ctx = Self {
            id: id.clone(),
            profile_name: profile_name.into(),
            created_at: Instant::now(),
            focused_page: None,
            state: LifeState::Ready,
        };
        (id, ctx)
    }
}

#[derive(Clone, Debug)]
pub struct PageCtx {
    pub id: PageId,
    pub session: SessionId,
    pub state: LifeState,
    pub main_frame: Option<FrameId>,
    pub focused_frame: Option<FrameId>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub last_active_at: Instant,
    pub health: PageHealth,
}

impl PageCtx {
    pub fn new(session: SessionId) -> (PageId, Self) {
        let id = PageId::new();
        let ctx = Self {
            id: id.clone(),
            session,
            state: LifeState::Init,
            main_frame: None,
            focused_frame: None,
            url: None,
            title: None,
            last_active_at: Instant::now(),
            health: PageHealth::default(),
        };
        (id, ctx)
    }
}

#[derive(Clone, Debug)]
pub struct FrameCtx {
    pub id: FrameId,
    pub page: PageId,
    pub parent: Option<FrameId>,
    pub children: Vec<FrameId>,
    pub state: LifeState,
    pub is_main: bool,
}

impl FrameCtx {
    pub fn new(page: PageId, parent: Option<FrameId>, is_main: bool) -> (FrameId, Self) {
        let id = FrameId::new();
        let ctx = Self {
            id: id.clone(),
            page,
            parent,
            children: Vec::new(),
            state: LifeState::Init,
            is_main,
        };
        (id, ctx)
    }
}
