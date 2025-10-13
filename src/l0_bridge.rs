use std::sync::Arc;

use cdp_adapter::{event_bus as cdp_event_bus, events::RawEvent};
use dashmap::DashMap;
use network_tap_light::{NetworkSnapshot, NetworkSummary, NetworkTapLight, PageId as TapPageId};
use soulbrowser_core_types::{FrameId, PageId, SessionId};
use soulbrowser_registry::{Registry, RegistryImpl};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use soulbrowser_state_center::StateCenter;

use crate::errors::SoulBrowserError;

#[derive(Clone)]
pub struct L0Handles {
    #[allow(dead_code)]
    pub cdp_sender: cdp_adapter::EventBus,
    #[allow(dead_code)]
    pub network_tap: Arc<NetworkTapLight>,
}

pub struct L0Bridge {
    cdp_task: JoinHandle<()>,
    network_task: JoinHandle<()>,
    #[allow(dead_code)]
    mapping: Arc<DashMap<cdp_adapter::ids::PageId, PageId>>,
    #[allow(dead_code)]
    network_tap: Arc<NetworkTapLight>,
    #[allow(dead_code)]
    cdp_sender: cdp_adapter::EventBus,
}

impl L0Bridge {
    pub fn new(
        registry: Arc<RegistryImpl>,
        _state_center: Arc<dyn StateCenter>,
        default_session: SessionId,
    ) -> (Self, L0Handles) {
        let mapping: Arc<DashMap<cdp_adapter::ids::PageId, PageId>> = Arc::new(DashMap::new());
        let tap_mapping: Arc<DashMap<network_tap_light::PageId, PageId>> = Arc::new(DashMap::new());
        let cdp_to_tap: Arc<DashMap<cdp_adapter::ids::PageId, network_tap_light::PageId>> =
            Arc::new(DashMap::new());
        let frames: Arc<DashMap<cdp_adapter::ids::FrameId, FrameId>> = Arc::new(DashMap::new());

        let (network_tap, mut network_rx) = NetworkTapLight::new(256);
        let registry_for_network = Arc::clone(&registry);
        let mapping_for_network: Arc<DashMap<TapPageId, PageId>> = Arc::clone(&tap_mapping);
        let network_tap_arc = Arc::new(network_tap);
        let network_tap_task_arc = Arc::clone(&network_tap_arc);
        let network_tap_for_cdp = Arc::clone(&network_tap_arc);

        let (cdp_sender, mut cdp_rx) = cdp_event_bus(256);
        let registry_for_task = Arc::clone(&registry);
        let mapping_for_task = Arc::clone(&mapping);
        let frames_for_task = Arc::clone(&frames);
        let tap_mapping_for_task = Arc::clone(&tap_mapping);
        let cdp_to_tap_for_task = Arc::clone(&cdp_to_tap);

        let cdp_task = tokio::spawn(async move {
            while let Ok(event) = cdp_rx.recv().await {
                if let Err(err) = handle_cdp_event(
                    &registry_for_task,
                    &mapping_for_task,
                    &frames_for_task,
                    &tap_mapping_for_task,
                    &cdp_to_tap_for_task,
                    &default_session,
                    &network_tap_for_cdp,
                    event,
                )
                .await
                {
                    warn!(target: "l0-bridge", "failed to handle cdp event: {err}");
                }
            }
        });
        let network_task = tokio::spawn(async move {
            loop {
                match network_rx.recv().await {
                    Ok(summary) => {
                        if let Some(page_id) = mapping_for_network
                            .get(&summary.page)
                            .map(|entry| entry.clone())
                        {
                            if let Err(err) = registry_for_network.apply_network_snapshot(
                                &page_id,
                                &NetworkSnapshot {
                                    req: summary.req,
                                    res2xx: summary.res2xx,
                                    res4xx: summary.res4xx,
                                    res5xx: summary.res5xx,
                                    inflight: summary.inflight,
                                    quiet: summary.quiet,
                                    window_ms: summary.window_ms,
                                    since_last_activity_ms: summary.since_last_activity_ms,
                                },
                            ) {
                                warn!(target: "l0-bridge", "failed to apply network snapshot: {err}");
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(target: "l0-bridge", skipped, "network summary channel lagged");
                    }
                }
            }
        });

        (
            Self {
                cdp_task,
                network_task,
                mapping,
                network_tap: Arc::clone(&network_tap_task_arc),
                cdp_sender: cdp_sender.clone(),
            },
            L0Handles {
                cdp_sender,
                network_tap: network_tap_task_arc,
            },
        )
    }
}

impl Drop for L0Bridge {
    fn drop(&mut self) {
        self.cdp_task.abort();
        self.network_task.abort();
    }
}

async fn handle_cdp_event(
    registry: &Arc<RegistryImpl>,
    mapping: &DashMap<cdp_adapter::ids::PageId, PageId>,
    frames: &DashMap<cdp_adapter::ids::FrameId, FrameId>,
    tap_mapping: &DashMap<network_tap_light::PageId, PageId>,
    cdp_to_tap: &DashMap<cdp_adapter::ids::PageId, network_tap_light::PageId>,
    default_session: &SessionId,
    network_tap: &Arc<NetworkTapLight>,
    event: RawEvent,
) -> Result<(), SoulBrowserError> {
    match event {
        RawEvent::PageLifecycle {
            page, frame, phase, ..
        } => {
            let phase_lower = phase.to_ascii_lowercase();
            match phase_lower.as_str() {
                "opened" | "open" => {
                    let pid_value = if let Some(entry) = mapping.get(&page) {
                        entry.clone()
                    } else {
                        let pid =
                            registry
                                .page_open(default_session.clone())
                                .await
                                .map_err(|err| {
                                    SoulBrowserError::internal(&format!(
                                        "registry page open failed: {err}"
                                    ))
                                })?;
                        mapping.insert(page, pid.clone());
                        let tap_page = TapPageId(page.0);
                        if let Err(err) = network_tap.enable(tap_page).await {
                            warn!(target: "l0-bridge", ?err, "failed to enable network tap for page");
                        }
                        tap_mapping.insert(tap_page, pid.clone());
                        cdp_to_tap.insert(page, tap_page);
                        pid
                    };
                    registry.page_focus(pid_value).await.map_err(|err| {
                        SoulBrowserError::internal(&format!("page focus failed: {err}"))
                    })?;
                }
                "closed" | "close" => {
                    if let Some((cdp_page, pid)) = mapping.remove(&page) {
                        let _ = registry.page_close(pid.clone()).await;
                        mapping.remove(&cdp_page);
                        if let Some((_, tap_page)) = cdp_to_tap.remove(&cdp_page) {
                            tap_mapping.remove(&tap_page);
                            let _ = network_tap.disable(tap_page).await;
                        }
                    }
                }
                "focus" | "focused" => {
                    if let Some(pid) = mapping.get(&page) {
                        if let Some(frame_id) =
                            frame.and_then(|fid| frames.get(&fid).map(|entry| entry.clone()))
                        {
                            registry
                                .frame_focus(pid.clone(), frame_id)
                                .await
                                .map_err(|err| {
                                    SoulBrowserError::internal(&format!(
                                        "frame focus failed: {err}"
                                    ))
                                })?;
                        } else {
                            registry.page_focus(pid.clone()).await.map_err(|err| {
                                SoulBrowserError::internal(&format!("page focus failed: {err}"))
                            })?;
                        }
                    }
                }
                "frame_attached" => {
                    if let (Some(pid), Some(fid)) = (mapping.get(&page), frame) {
                        let is_main = frames.is_empty();
                        let frame_id = FrameId::new();
                        frames.insert(fid, frame_id.clone());
                        let pid_value = pid.clone();
                        registry
                            .frame_attached(&pid_value, None, is_main)
                            .map_err(|err| {
                                SoulBrowserError::internal(&format!("frame attach failed: {err}"))
                            })?;
                    }
                }
                "frame_detached" => {
                    if let Some(fid) = frame.and_then(|fid| frames.remove(&fid).map(|(_, v)| v)) {
                        registry.frame_detached(&fid).map_err(|err| {
                            SoulBrowserError::internal(&format!("frame detach failed: {err}"))
                        })?;
                    }
                }
                _ => {
                    debug!(target: "l0-bridge", phase, "unhandled page lifecycle phase");
                }
            }
        }
        RawEvent::NetworkSummary {
            page,
            req,
            res2xx,
            res4xx,
            res5xx,
            inflight,
            quiet,
            window_ms,
            since_last_activity_ms,
        } => {
            if let Some(pid) = mapping.get(&page) {
                let snapshot = NetworkSnapshot {
                    req,
                    res2xx,
                    res4xx,
                    res5xx,
                    inflight,
                    quiet,
                    window_ms,
                    since_last_activity_ms,
                };
                registry
                    .apply_network_snapshot(&pid, &snapshot)
                    .map_err(|err| {
                        SoulBrowserError::internal(&format!("network summary apply failed: {err}"))
                    })?;
                if let Some(tap_page) = cdp_to_tap.get(&page) {
                    let _ = network_tap.publish_summary(NetworkSummary {
                        page: *tap_page,
                        window_ms: snapshot.window_ms,
                        req,
                        res2xx,
                        res4xx,
                        res5xx,
                        inflight,
                        quiet,
                        since_last_activity_ms: snapshot.since_last_activity_ms,
                    });
                }
            }
        }
        RawEvent::Error { message, .. } => {
            warn!(target: "l0-bridge", message, "cdp error event received");
        }
    }
    Ok(())
}
