use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use soulbrowser_core_types::{ActionId, FrameId, PageId};

/// Snapshot granularity level.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapLevel {
    Light,
    Full,
}

/// Snapshot context provided by the producer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapCtx {
    pub action: ActionId,
    pub page: PageId,
    pub frame: FrameId,
    pub ts_wall: DateTime<Utc>,
    pub ts_mono: u128,
}

impl SnapCtx {
    pub fn new(
        action: ActionId,
        page: PageId,
        frame: FrameId,
        ts_wall: DateTime<Utc>,
        ts_mono: u128,
    ) -> Self {
        Self {
            action,
            page,
            frame,
            ts_wall,
            ts_mono,
        }
    }
}

/// Structure snapshot persisted on disk.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructSnap {
    pub id: String,
    pub kind: String,
    pub level: SnapLevel,
    pub page: PageId,
    pub frame: FrameId,
    pub action: ActionId,
    pub ts_wall: DateTime<Utc>,
    pub ts_mono: u128,
    pub dom_zstd: Option<Vec<u8>>,
    pub ax_zstd: Option<Vec<u8>>,
    pub meta: StructMeta,
}

/// Metadata describing structure payloads after redaction/compression.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StructMeta {
    pub node_count: u32,
    pub masked: bool,
    pub bytes: u64,
    #[serde(default)]
    pub masked_fields: Vec<String>,
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Pixel clip representation referencing small thumbnails.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PixClip {
    pub id: String,
    pub page: PageId,
    pub frame: FrameId,
    pub action: ActionId,
    pub rect: Rect,
    pub thumb: PixThumb,
    pub meta: PixMeta,
    #[serde(default)]
    pub compressed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PixThumb {
    pub w: u32,
    pub h: u32,
    pub bytes: Vec<u8>,
    pub fmt: PixFmt,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PixFmt {
    Jpeg,
    Png,
    Webp,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PixMeta {
    pub masked: bool,
    pub bytes: u64,
    pub origin: Option<String>,
    #[serde(default)]
    pub compression: Option<String>,
}

/// Action binding reference.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapRef {
    pub action: ActionId,
    pub page: PageId,
    pub frame: FrameId,
    pub struct_id: Option<String>,
    pub pix_ids: Vec<String>,
    pub ttl_at: DateTime<Utc>,
}

/// Minimal replay payload exposing structural evidence.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReplayBundle {
    #[serde(default)]
    pub struct_id: Option<String>,
    #[serde(default)]
    pub pix_ids: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

/// Sweep statistics for TTL/quota cleanup actions.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SweepStats {
    pub expired: usize,
    pub removed_struct: usize,
    pub removed_pix: usize,
}

/// Binding request with optional override TTL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BindRequest {
    pub action: ActionId,
    pub page: PageId,
    pub frame: FrameId,
    pub struct_id: Option<String>,
    pub pix_ids: Vec<String>,
    pub ttl: Option<Duration>,
}

/// Lightweight representation of raw DOM/AX input prior to encoding.
#[derive(Clone, Debug)]
pub struct DomAxRaw {
    pub dom: serde_json::Value,
    pub ax: serde_json::Value,
}

/// Source image buffer delivered alongside region metadata.
#[derive(Clone, Debug)]
pub struct ImageBuf {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub stride: usize,
}
