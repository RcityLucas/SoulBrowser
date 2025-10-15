use serde_json::json;
use soulbrowser_core_types::FrameId;

use crate::model::{AnchorDescriptor, ResolveHint};

pub fn from_hint(_hint: &ResolveHint) -> Vec<AnchorDescriptor> {
    vec![AnchorDescriptor {
        strategy: "stub".into(),
        value: json!({}),
        frame_id: FrameId("root".into()),
        confidence: 0.5,
        backend_node_id: None,
        geometry: None,
    }]
}
