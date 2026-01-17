use std::sync::Arc;

use cdp_adapter::{event_bus as cdp_event_bus, events::RawEvent, Cdp, CdpAdapter};
use dashmap::DashMap;
use network_tap_light::{NetworkSnapshot, NetworkTapLight, PageId as TapPageId, TapEvent};
use soulbrowser_core_types::{FrameId, PageId, SessionId};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{debug, warn};

use permissions_broker::{
    config::{
        default_permission_map, default_policy_file, load_permission_map_from_path,
        load_policy_from_path,
    },
    Broker, CdpPermissionTransport, PermissionsBroker,
};
use url::Url;

use soulbrowser_registry::{metrics as registry_metrics, Registry, RegistryImpl};
use soulbrowser_scheduler::{RouteEvent, RouteEventSender};
use soulbrowser_state_center::{RegistryAction, RegistryEvent, StateCenter, StateEvent};

use crate::errors::SoulBrowserError;

#[derive(Clone)]
pub struct L0Handles {
    #[allow(dead_code)]
    pub cdp_sender: cdp_adapter::EventBus,
    #[allow(dead_code)]
    pub network_tap: Arc<NetworkTapLight>,
    #[allow(dead_code)]
    pub permissions: Arc<PermissionsBroker>,
}

impl L0Handles {
    /// Attach a CDP adapter so the permissions broker can issue Browser.setPermission calls.
    pub async fn attach_cdp_adapter(&self, adapter: &Arc<CdpAdapter>) {
        let adapter_arc: Arc<CdpAdapter> = Arc::clone(adapter);
        let dyn_adapter: Arc<dyn Cdp + Send + Sync> = adapter_arc;
        let transport = Arc::new(CdpPermissionTransport::new(dyn_adapter));
        self.permissions.set_transport(transport).await;
    }
}

pub struct L0Bridge {
    cdp_task: JoinHandle<()>,
    network_task: JoinHandle<()>,
    quiet_task: JoinHandle<()>,
    #[allow(dead_code)]
    mapping: Arc<DashMap<cdp_adapter::ids::PageId, PageId>>,
    #[allow(dead_code)]
    network_tap: Arc<NetworkTapLight>,
    #[allow(dead_code)]
    cdp_sender: cdp_adapter::EventBus,
    #[allow(dead_code)]
    permissions: Arc<PermissionsBroker>,
    #[allow(dead_code)]
    page_origins: Arc<DashMap<PageId, String>>,
    #[allow(dead_code)]
    page_sessions: Arc<DashMap<PageId, SessionId>>,
    #[allow(dead_code)]
    state_center: Arc<dyn StateCenter + Send + Sync>,
    route_events: Option<Arc<RouteEventSender>>,
}

const PERMISSIONS_POLICY_PATH: &str = "config/permissions/policy.json";
const PERMISSIONS_MAP_PATH: &str = "config/permissions/map.json";

impl L0Bridge {
    pub async fn new(
        registry: Arc<RegistryImpl>,
        state_center: Arc<dyn StateCenter + Send + Sync>,
        default_session: SessionId,
        route_events: Option<Arc<RouteEventSender>>,
    ) -> (Self, L0Handles) {
        let mapping: Arc<DashMap<cdp_adapter::ids::PageId, PageId>> = Arc::new(DashMap::new());
        let tap_mapping: Arc<DashMap<network_tap_light::PageId, PageId>> = Arc::new(DashMap::new());
        let cdp_to_tap: Arc<DashMap<cdp_adapter::ids::PageId, network_tap_light::PageId>> =
            Arc::new(DashMap::new());
        let frames: Arc<DashMap<cdp_adapter::ids::FrameId, FrameId>> = Arc::new(DashMap::new());

        let permissions = Self::init_permissions_broker().await;
        let page_origins: Arc<DashMap<PageId, String>> = Arc::new(DashMap::new());
        let page_sessions: Arc<DashMap<PageId, SessionId>> = Arc::new(DashMap::new());
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
        let permissions_for_task = Arc::clone(&permissions);
        let origins_for_task = Arc::clone(&page_origins);
        let page_sessions_for_task = Arc::clone(&page_sessions);
        let state_center_for_task = Arc::clone(&state_center);
        let route_events_for_task = route_events.clone();

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
                    &permissions_for_task,
                    &origins_for_task,
                    &page_sessions_for_task,
                    &state_center_for_task,
                    &route_events_for_task,
                    event,
                )
                .await
                {
                    warn!(target: "l0-bridge", "failed to handle cdp event: {err}");
                }
            }
        });
        let timer_tap = Arc::clone(&network_tap_arc);
        let quiet_task = tokio::spawn(async move {
            loop {
                sleep(Duration::from_millis(100)).await;
                timer_tap.evaluate_timeouts().await;
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
                quiet_task,
                mapping,
                network_tap: Arc::clone(&network_tap_task_arc),
                cdp_sender: cdp_sender.clone(),
                permissions: Arc::clone(&permissions),
                page_origins: Arc::clone(&page_origins),
                page_sessions: Arc::clone(&page_sessions),
                state_center: Arc::clone(&state_center),
                route_events,
            },
            L0Handles {
                cdp_sender,
                network_tap: network_tap_task_arc,
                permissions,
            },
        )
    }

    async fn init_permissions_broker() -> Arc<PermissionsBroker> {
        let broker = Arc::new(PermissionsBroker::new());

        let map = match load_permission_map_from_path(PERMISSIONS_MAP_PATH) {
            Ok(map) => map,
            Err(err) => {
                warn!(
                    target = "l0-bridge",
                    path = PERMISSIONS_MAP_PATH,
                    ?err,
                    "failed to load permission map; using default"
                );
                default_permission_map()
            }
        };
        broker.set_permission_map(map).await;

        let policy = match load_policy_from_path(PERMISSIONS_POLICY_PATH) {
            Ok(policy) => policy,
            Err(err) => {
                warn!(
                    target = "l0-bridge",
                    path = PERMISSIONS_POLICY_PATH,
                    ?err,
                    "failed to load permission policy; using default"
                );
                default_policy_file()
            }
        };

        if let Err(err) = broker.load_policy(policy).await {
            warn!(
                target = "l0-bridge",
                ?err,
                "failed to load permission policy into broker"
            );
        }

        broker
    }
}

impl Drop for L0Bridge {
    fn drop(&mut self) {
        self.cdp_task.abort();
        self.network_task.abort();
        self.quiet_task.abort();
    }
}

fn extract_origin(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let scheme = parsed.scheme();
    let default_port = parsed.port_or_known_default();
    match parsed.port() {
        Some(port) if Some(port) != default_port => Some(format!("{}://{}:{}", scheme, host, port)),
        _ => Some(format!("{}://{}", scheme, host)),
    }
}

fn page_session_or_default(
    page_sessions: &DashMap<PageId, SessionId>,
    page: &PageId,
    fallback: &SessionId,
) -> SessionId {
    page_sessions
        .get(page)
        .map(|entry| entry.clone())
        .unwrap_or_else(|| fallback.clone())
}

fn emit_route_event(
    route_events: &Option<Arc<RouteEventSender>>,
    session: SessionId,
    page: PageId,
    frame: Option<FrameId>,
) {
    if let Some(sender) = route_events {
        let _ = sender.send(RouteEvent {
            session,
            page,
            frame,
        });
    }
}

fn should_apply_permissions(
    page_origins: &DashMap<PageId, String>,
    page: &PageId,
    origin: &str,
) -> bool {
    page_origins
        .get(page)
        .map(|existing| existing.value() != origin)
        .unwrap_or(true)
}

async fn handle_cdp_event(
    registry: &Arc<RegistryImpl>,
    mapping: &DashMap<cdp_adapter::ids::PageId, PageId>,
    frames: &DashMap<cdp_adapter::ids::FrameId, FrameId>,
    tap_mapping: &DashMap<network_tap_light::PageId, PageId>,
    cdp_to_tap: &DashMap<cdp_adapter::ids::PageId, network_tap_light::PageId>,
    default_session: &SessionId,
    network_tap: &Arc<NetworkTapLight>,
    permissions: &Arc<PermissionsBroker>,
    page_origins: &Arc<DashMap<PageId, String>>,
    page_sessions: &Arc<DashMap<PageId, SessionId>>,
    state_center: &Arc<dyn StateCenter + Send + Sync>,
    route_events: &Option<Arc<RouteEventSender>>,
    event: RawEvent,
) -> Result<(), SoulBrowserError> {
    match event {
        RawEvent::PageLifecycle {
            page,
            frame,
            parent,
            opener,
            phase,
            ..
        } => {
            let phase_lower = phase.to_ascii_lowercase();
            match phase_lower.as_str() {
                "opened" | "open" => {
                    let mut opened_page: Option<(SessionId, PageId)> = None;
                    let pid_value = if let Some(entry) = mapping.get(&page) {
                        entry.clone()
                    } else {
                        let session_to_use = opener
                            .and_then(|opener_page| {
                                mapping.get(&opener_page).map(|entry| entry.clone())
                            })
                            .and_then(|parent_pid| {
                                page_sessions
                                    .get(&parent_pid)
                                    .map(|entry| entry.value().clone())
                            })
                            .unwrap_or_else(|| default_session.clone());
                        let pid =
                            registry
                                .page_open(session_to_use.clone())
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
                        page_sessions.insert(pid.clone(), session_to_use.clone());
                        opened_page = Some((session_to_use, pid.clone()));
                        pid
                    };
                    registry.page_focus(pid_value).await.map_err(|err| {
                        SoulBrowserError::internal(&format!("page focus failed: {err}"))
                    })?;
                    if let Some((session, page)) = opened_page {
                        emit_route_event(route_events, session, page, None);
                    }
                }
                "closed" | "close" => {
                    if let Some((cdp_page, pid)) = mapping.remove(&page) {
                        let _ = registry.page_close(pid.clone()).await;
                        mapping.remove(&cdp_page);
                        page_origins.remove(&pid);
                        page_sessions.remove(&pid);
                        if let Some((_, tap_page)) = cdp_to_tap.remove(&cdp_page) {
                            tap_mapping.remove(&tap_page);
                            let _ = network_tap.disable(tap_page).await;
                        }
                    }
                }
                "focus" | "focused" => {
                    if let Some(pid_entry) = mapping.get(&page) {
                        let pid_value = pid_entry.clone();
                        if let Some(frame_id) =
                            frame.and_then(|fid| frames.get(&fid).map(|entry| entry.clone()))
                        {
                            registry
                                .frame_focus(pid_value.clone(), frame_id.clone())
                                .await
                                .map_err(|err| {
                                    SoulBrowserError::internal(&format!(
                                        "frame focus failed: {err}"
                                    ))
                                })?;
                            let session =
                                page_session_or_default(page_sessions, &pid_value, default_session);
                            emit_route_event(route_events, session, pid_value, Some(frame_id));
                        } else {
                            registry
                                .page_focus(pid_value.clone())
                                .await
                                .map_err(|err| {
                                    SoulBrowserError::internal(&format!("page focus failed: {err}"))
                                })?;
                            let session =
                                page_session_or_default(page_sessions, &pid_value, default_session);
                            emit_route_event(route_events, session, pid_value, None);
                        }
                    }
                }
                "frame_attached" => {
                    if let (Some(pid_entry), Some(fid)) = (mapping.get(&page), frame) {
                        let pid_value = pid_entry.clone();
                        let parent_frame = parent.and_then(|parent_fid| {
                            frames.get(&parent_fid).map(|entry| entry.clone())
                        });
                        let is_main = parent_frame.is_none();
                        let new_frame = registry
                            .frame_attached(&pid_value, parent_frame.clone(), is_main)
                            .map_err(|err| {
                                SoulBrowserError::internal(&format!("frame attach failed: {err}"))
                            })?;
                        frames.insert(fid, new_frame.clone());
                        if is_main {
                            let session =
                                page_session_or_default(page_sessions, &pid_value, default_session);
                            emit_route_event(
                                route_events,
                                session,
                                pid_value.clone(),
                                Some(new_frame),
                            );
                        }
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
        RawEvent::PageNavigated { page, url, ts } => {
            if let Some(entry) = mapping.get(&page) {
                let registry_page = entry.value().clone();
                registry.update_page_url(&registry_page, url.clone());
                let session_id = page_sessions
                    .get(&registry_page)
                    .map(|entry| entry.value().clone());
                drop(entry);

                if let Some(origin) = extract_origin(&url) {
                    if should_apply_permissions(page_origins.as_ref(), &registry_page, &origin) {
                        match permissions.apply_policy(&origin).await {
                            Ok(decision) => {
                                if let Some(session) = session_id {
                                    let note = format!(
                                        "origin={} allowed={:?} denied={:?} missing={:?} ttl={:?} ts={}",
                                        origin,
                                        decision.allowed,
                                        decision.denied,
                                        decision.missing,
                                        decision.ttl_ms,
                                        ts
                                    );
                                    let event = RegistryEvent::new(
                                        RegistryAction::PermissionsApplied,
                                        Some(session),
                                        Some(registry_page.clone()),
                                        None,
                                        Some(note),
                                    );
                                    if let Err(err) =
                                        state_center.append(StateEvent::registry(event)).await
                                    {
                                        warn!(
                                            target = "l0-bridge",
                                            ?err,
                                            "failed to append permissions event to state center"
                                        );
                                    }
                                }
                                registry_metrics::record_permissions_applied(&origin);
                                page_origins.insert(registry_page.clone(), origin.clone());
                            }
                            Err(err) => {
                                warn!(
                                    target = "l0-bridge",
                                    origin,
                                    ?err,
                                    "failed to apply permissions via broker"
                                );
                            }
                        }
                    }
                } else {
                    warn!(
                        target = "l0-bridge",
                        url, "unable to derive origin for permissions"
                    );
                }
            }
        }
        RawEvent::NetworkActivity { page, signal } => {
            if let Some(tap_page) = cdp_to_tap.get(&page) {
                let tap_event = match signal {
                    cdp_adapter::events::NetworkSignal::RequestWillBeSent => {
                        TapEvent::RequestWillBeSent
                    }
                    cdp_adapter::events::NetworkSignal::ResponseReceived { status } => {
                        TapEvent::ResponseReceived { status }
                    }
                    cdp_adapter::events::NetworkSignal::LoadingFinished => {
                        TapEvent::LoadingFinished
                    }
                    cdp_adapter::events::NetworkSignal::LoadingFailed => TapEvent::LoadingFailed,
                };
                if let Err(err) = network_tap.ingest(*tap_page, tap_event).await {
                    warn!(target: "l0-bridge", ?err, "network tap ingest failed");
                }
            }
        }
        RawEvent::NetworkSummary { .. } => {
            debug!(target: "l0-bridge", "legacy network summary ignored");
        }
        RawEvent::Error { message, .. } => {
            warn!(target: "l0-bridge", message, "cdp error event received");
        }
    }
    Ok(())
}
