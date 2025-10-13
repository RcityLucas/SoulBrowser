//! Adapter registry keeping track of active pages/sessions/targets.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::ids::{PageId, SessionId};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetContext {
    pub session_id: SessionId,
    pub target_id: Option<String>,
    pub cdp_session: Option<String>,
    pub recent_url: Option<String>,
}

/// Concurrent registry for pages and sessions.
pub struct Registry {
    pages: DashMap<PageId, TargetContext>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            pages: DashMap::new(),
        }
    }

    pub fn insert_page(
        &self,
        page: PageId,
        session: SessionId,
        target_id: Option<String>,
        cdp_session: Option<String>,
    ) {
        let ctx = TargetContext {
            session_id: session,
            target_id,
            cdp_session,
            recent_url: None,
        };
        self.pages.insert(page, ctx);
    }

    pub fn remove_page(&self, page: &PageId) {
        self.pages.remove(page);
    }

    pub fn get(&self, page: &PageId) -> Option<TargetContext> {
        self.pages.get(page).map(|entry| entry.value().clone())
    }

    pub fn iter(&self) -> Vec<(PageId, TargetContext)> {
        self.pages
            .iter()
            .map(|kv| (*kv.key(), kv.value().clone()))
            .collect()
    }

    pub fn set_recent_url(&self, page: &PageId, url: String) {
        if let Some(mut entry) = self.pages.get_mut(page) {
            entry.recent_url = Some(url);
        }
    }

    pub fn set_cdp_session(&self, page: &PageId, session: String) {
        if let Some(mut entry) = self.pages.get_mut(page) {
            entry.cdp_session = Some(session);
        }
    }

    pub fn get_cdp_session(&self, page: &PageId) -> Option<String> {
        self.pages
            .get(page)
            .and_then(|entry| entry.cdp_session.clone())
    }
}
