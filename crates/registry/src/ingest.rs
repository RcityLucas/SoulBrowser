use crate::api::Registry;
use network_tap_light::NetworkSnapshot;
use soulbrowser_core_types::{FrameId, PageId, SessionId};
use soulbrowser_event_bus::{EventBus, InMemoryBus};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::warn;

use crate::state::RegistryImpl;

#[derive(Clone, Debug)]
pub enum RegistryEvent {
    PageFocus {
        page: PageId,
    },
    PageClose {
        page: PageId,
    },
    PageOpen {
        session: SessionId,
    },
    FrameFocus {
        page: PageId,
        frame: FrameId,
    },
    FrameAttached {
        page: PageId,
        parent: Option<FrameId>,
        is_main: bool,
    },
    FrameDetached {
        frame: FrameId,
    },
    NetworkSummary {
        page: PageId,
        snapshot: NetworkSnapshot,
    },
    HealthProbeTick,
}

pub struct IngestHandle {
    task: JoinHandle<()>,
    health_task: JoinHandle<()>,
    health_interval_ms: Arc<AtomicU64>,
}

impl IngestHandle {
    pub fn spawn(
        bus: Arc<InMemoryBus<RegistryEvent>>,
        registry: Arc<RegistryImpl>,
        health_interval_ms: Arc<AtomicU64>,
    ) -> Self {
        let mut rx = bus.subscribe();
        let registry_for_bus = Arc::clone(&registry);
        let task = tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                if let Err(err) = handle_event(&registry_for_bus, event).await {
                    warn!("registry ingest error: {err}");
                }
            }
        });
        let registry_clone = Arc::clone(&registry);
        let interval_clone = Arc::clone(&health_interval_ms);
        let health_task = tokio::spawn(async move {
            loop {
                let interval = interval_clone.load(Ordering::Relaxed);
                if interval == 0 {
                    tokio::time::sleep(Duration::from_millis(1_000)).await;
                    continue;
                }
                tokio::time::sleep(Duration::from_millis(interval)).await;
                registry_clone.health_probe_tick();
            }
        });
        Self {
            task,
            health_task,
            health_interval_ms,
        }
    }

    pub async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
        self.health_task.abort();
        let _ = self.health_task.await;
    }

    pub fn health_interval_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.health_interval_ms)
    }
}

async fn handle_event(
    registry: &Arc<RegistryImpl>,
    event: RegistryEvent,
) -> Result<(), soulbrowser_core_types::SoulError> {
    match event {
        RegistryEvent::PageFocus { page } => registry.page_focus(page).await,
        RegistryEvent::PageClose { page } => registry.page_close(page).await,
        RegistryEvent::PageOpen { session } => registry.page_open(session).await.map(|_| ()),
        RegistryEvent::FrameFocus { page, frame } => registry.frame_focus(page, frame).await,
        RegistryEvent::FrameAttached {
            page,
            parent,
            is_main,
        } => {
            registry.frame_attached(&page, parent, is_main)?;
            Ok(())
        }
        RegistryEvent::FrameDetached { frame } => {
            registry.frame_detached(&frame)?;
            Ok(())
        }
        RegistryEvent::NetworkSummary { page, snapshot } => {
            registry.apply_network_snapshot(&page, &snapshot)?;
            Ok(())
        }
        RegistryEvent::HealthProbeTick => {
            registry.health_probe_tick();
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LifeState;
    use network_tap_light::NetworkSnapshot;
    use std::sync::atomic::AtomicU64;

    fn wait_ms() -> u64 {
        10
    }

    fn health_interval() -> Arc<AtomicU64> {
        Arc::new(AtomicU64::new(100))
    }

    #[tokio::test]
    async fn page_focus_event_updates_registry() {
        let bus = InMemoryBus::new(16);
        let registry = Arc::new(RegistryImpl::new());
        let _handle = IngestHandle::spawn(bus.clone(), registry.clone(), health_interval());

        let session = registry.session_create("user").await.unwrap();
        let page = registry.page_open(session.clone()).await.unwrap();

        bus.publish(RegistryEvent::PageFocus { page: page.clone() })
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms())).await;

        let exec = registry.route_resolve(None).await.unwrap();
        assert_eq!(exec.page, page);
    }

    #[tokio::test]
    async fn frame_attach_and_detach_events_update_tree() {
        let bus = InMemoryBus::new(16);
        let registry = Arc::new(RegistryImpl::new());
        let _handle = IngestHandle::spawn(bus.clone(), registry.clone(), health_interval());

        let session = registry.session_create("user").await.unwrap();
        let page = registry.page_open(session.clone()).await.unwrap();

        let main_frame = {
            let page_ctx = registry.ensure_page(&page).unwrap();
            let guard = page_ctx.read();
            guard.main_frame.clone().unwrap()
        };

        bus.publish(RegistryEvent::FrameAttached {
            page: page.clone(),
            parent: Some(main_frame.clone()),
            is_main: false,
        })
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms())).await;

        let child_count = {
            let main_ctx = registry.ensure_frame(&main_frame).unwrap();
            let len = main_ctx.read().children.len();
            len
        };
        assert_eq!(child_count, 1);

        let child_id = {
            let main_ctx = registry.ensure_frame(&main_frame).unwrap();
            let child = main_ctx.read().children[0].clone();
            child
        };

        bus.publish(RegistryEvent::FrameDetached { frame: child_id })
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms())).await;

        let main_ctx = registry.ensure_frame(&main_frame).unwrap();
        assert!(main_ctx.read().children.is_empty());
    }

    #[tokio::test]
    async fn frame_focus_event_routes() {
        let bus = InMemoryBus::new(16);
        let registry = Arc::new(RegistryImpl::new());
        let _handle = IngestHandle::spawn(bus.clone(), registry.clone(), health_interval());

        let session = registry.session_create("user").await.unwrap();
        let page = registry.page_open(session.clone()).await.unwrap();

        let frame = {
            let page_ctx = registry.ensure_page(&page).unwrap();
            let focused = page_ctx.read().focused_frame.clone().unwrap();
            focused
        };

        bus.publish(RegistryEvent::FrameFocus {
            page: page.clone(),
            frame: frame.clone(),
        })
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms())).await;

        let exec = registry
            .route_resolve(Some(soulbrowser_core_types::RoutingHint {
                page: Some(page.clone()),
                ..Default::default()
            }))
            .await
            .unwrap();
        assert_eq!(exec.frame, frame);
    }

    #[tokio::test]
    async fn network_summary_updates_health() {
        let bus = InMemoryBus::new(16);
        let registry = Arc::new(RegistryImpl::new());
        let _handle = IngestHandle::spawn(bus.clone(), registry.clone(), health_interval());

        let session = registry.session_create("user").await.unwrap();
        let page = registry.page_open(session.clone()).await.unwrap();

        let summary = NetworkSnapshot {
            req: 25,
            res2xx: 20,
            res4xx: 3,
            res5xx: 2,
            inflight: 1,
            quiet: false,
            window_ms: 1_000,
            since_last_activity_ms: 50,
        };

        bus.publish(RegistryEvent::NetworkSummary {
            page: page.clone(),
            snapshot: summary.clone(),
        })
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms())).await;

        let page_ctx = registry.ensure_page(&page).unwrap();
        let guard = page_ctx.read();
        assert_eq!(guard.health.request_count, summary.req);
        assert!(!guard.health.quiet);
    }

    #[tokio::test]
    async fn page_close_event_cleans_state() {
        let bus = InMemoryBus::new(16);
        let registry = Arc::new(RegistryImpl::new());
        let _handle = IngestHandle::spawn(bus.clone(), registry.clone(), health_interval());

        let session = registry.session_create("user").await.unwrap();
        let page_a = registry.page_open(session.clone()).await.unwrap();
        let page_b = registry.page_open(session.clone()).await.unwrap();

        registry.page_focus(page_b.clone()).await.unwrap();

        bus.publish(RegistryEvent::PageClose {
            page: page_b.clone(),
        })
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms())).await;

        assert!(registry.pages.get(&page_b).is_none());
        let exec = registry.route_resolve(None).await.unwrap();
        assert_eq!(exec.page, page_a);

        let session_ctx = registry.ensure_session(&session).unwrap();
        assert_eq!(session_ctx.read().state, LifeState::Active);
    }
}
