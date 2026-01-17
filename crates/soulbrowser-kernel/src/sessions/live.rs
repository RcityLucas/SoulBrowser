use serde::Serialize;

use serde_json::Value;

use super::types::{LiveFramePayload, LiveOverlayEntry, SessionSnapshot, SessionStatus};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionLiveEvent {
    Snapshot {
        snapshot: SessionSnapshot,
    },
    Status {
        session_id: String,
        status: SessionStatus,
    },
    Frame {
        frame: LiveFramePayload,
    },
    Overlay {
        overlay: LiveOverlayEntry,
    },
    MessageState {
        session_id: String,
        state: Value,
    },
}
