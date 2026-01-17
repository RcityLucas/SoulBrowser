use soulbrowser_core_types::{FrameId, PageId, SessionId};
use tokio::sync::broadcast;

/// Event emitted when a session acquires a new authoritative page route.
#[derive(Clone, Debug)]
pub struct RouteEvent {
    pub session: SessionId,
    pub page: PageId,
    pub frame: Option<FrameId>,
}

pub type RouteEventSender = broadcast::Sender<RouteEvent>;
pub type RouteEventReceiver = broadcast::Receiver<RouteEvent>;

pub fn route_event_channel(buffer: usize) -> (RouteEventSender, RouteEventReceiver) {
    broadcast::channel(buffer.max(1))
}
