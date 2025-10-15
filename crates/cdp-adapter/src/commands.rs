//! Command parameter types exposed by the CDP adapter interface.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Specification for a DOM query operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuerySpec {
    pub selector: String,
    pub scope: QueryScope,
}

/// Query scope determines which portion of the document the adapter should inspect.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum QueryScope {
    Document,
    Frame(String),
}

/// Target for click or typing operations (L2 resolves concrete data; L0 only injects).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Anchor {
    pub backend_node_id: Option<u64>,
    pub x: f64,
    pub y: f64,
}

/// Specification for selecting an option from a `<select>` element.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectSpec {
    pub selector: String,
    pub value: String,
    #[serde(default)]
    pub match_label: bool,
}

/// Wait gate definitions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WaitGate {
    DomReady,
    NetworkQuiet { window_ms: u64, max_inflight: u32 },
    FrameStable { min_stable_ms: u64 },
}

/// Options for capturing screenshots.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScreenshotOptions {
    pub clip: Option<ScreenshotClip>,
    pub format: ScreenshotFormat,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScreenshotClip {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ScreenshotFormat {
    Png,
    Jpeg { quality: Option<u8> },
}

/// Placeholder for accessor types that will wrap DOM/AX snapshots.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotOptions {
    pub include_dom: bool,
    pub include_ax: bool,
}

/// Options for DOM snapshot capture.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DomSnapshotConfig {
    #[serde(default)]
    pub computed_style_whitelist: Vec<String>,
    #[serde(default)]
    pub include_event_listeners: bool,
    #[serde(default)]
    pub include_paint_order: bool,
    #[serde(default)]
    pub include_user_agent_shadow_tree: bool,
}

impl Default for DomSnapshotConfig {
    fn default() -> Self {
        Self {
            computed_style_whitelist: vec![
                "display".into(),
                "visibility".into(),
                "opacity".into(),
                "pointer-events".into(),
                "transform".into(),
            ],
            include_event_listeners: false,
            include_paint_order: false,
            include_user_agent_shadow_tree: false,
        }
    }
}

/// DOM snapshot payload returned by the adapter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DomSnapshotResult {
    pub documents: Vec<Value>,
    pub strings: Vec<String>,
    pub raw: Value,
}

/// Options for Accessibility tree snapshot capture.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AxSnapshotConfig {
    pub frame_id: Option<String>,
    pub max_depth: Option<u32>,
    #[serde(default)]
    pub fetch_relatives: bool,
}

/// AX snapshot payload returned by the adapter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AxSnapshotResult {
    pub nodes: Vec<Value>,
    pub tree_id: Option<String>,
    pub raw: Value,
}
