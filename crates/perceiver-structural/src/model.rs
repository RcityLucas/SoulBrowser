use serde_json::Value;
use soulbrowser_core_types::FrameId;

#[derive(Clone, Debug)]
pub struct AnchorGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug)]
pub enum ResolveHint {
    Css(String),
    Aria { role: String, name: Option<String> },
    Text { pattern: String },
    Backend(u64),
    Geometry { x: i32, y: i32, w: i32, h: i32 },
}

impl ResolveHint {
    pub fn cache_key(&self) -> String {
        match self {
            ResolveHint::Css(sel) => format!("css:{sel}"),
            ResolveHint::Aria { role, name } => {
                format!("aria:{role}:{:?}", name)
            }
            ResolveHint::Text { pattern } => format!("text:{pattern}"),
            ResolveHint::Backend(id) => format!("backend:{id}"),
            ResolveHint::Geometry { x, y, w, h } => format!("geom:{x}:{y}:{w}:{h}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AnchorDescriptor {
    pub strategy: String,
    pub value: Value,
    pub frame_id: FrameId,
    pub confidence: f32,
    pub backend_node_id: Option<u64>,
    pub geometry: Option<AnchorGeometry>,
}

#[derive(Clone, Debug)]
pub struct AnchorResolution {
    pub primary: AnchorDescriptor,
    pub candidates: Vec<AnchorDescriptor>,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct JudgeReport {
    pub ok: bool,
    pub reason: String,
    pub facts: Value,
}

#[derive(Clone, Debug)]
pub struct DomAxSnapshot {
    pub dom_raw: Value,
    pub ax_raw: Value,
}

#[derive(Clone, Debug)]
pub struct DomAxDiff {
    pub changes: Vec<Value>,
}

#[derive(Clone, Debug)]
pub struct SampledPair {
    pub dom: Value,
    pub ax: Value,
}
