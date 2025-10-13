//! Command parameter types exposed by the CDP adapter interface.

use serde::{Deserialize, Serialize};

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
