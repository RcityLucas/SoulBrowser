use std::time::{Instant, SystemTime};

use serde_json::Value;
use soulbrowser_core_types::{FrameId, PageId, SessionId};
use uuid::Uuid;

use crate::policy::ResolveOptions;

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
        SelectorOrHint::from(self).cache_key()
    }
}

impl SelectorOrHint {
    pub fn cache_key(&self) -> String {
        match self {
            SelectorOrHint::Css(sel) => format!("css:{sel}"),
            SelectorOrHint::Aria { role, name, state } => {
                format!("aria:{role}:{:?}:{:?}", name, state)
            }
            SelectorOrHint::Ax { role, name, value } => {
                format!("ax:{role}:{:?}:{:?}", name, value)
            }
            SelectorOrHint::Text { pattern, fuzzy } => {
                format!("text:{pattern}:{:?}", fuzzy)
            }
            SelectorOrHint::Attr { key, value } => format!("attr:{key}:{value}"),
            SelectorOrHint::Backend(id) => format!("backend:{id}"),
            SelectorOrHint::Geometry { x, y, w, h } => {
                format!("geom:{x}:{y}:{w}:{h}")
            }
            SelectorOrHint::Combo(items) => {
                let parts: Vec<String> = items.iter().map(|item| item.cache_key()).collect();
                format!("combo:[{}]", parts.join("|"))
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum SelectorOrHint {
    Css(String),
    Aria {
        role: String,
        name: Option<String>,
        state: Option<String>,
    },
    Ax {
        role: String,
        name: Option<String>,
        value: Option<String>,
    },
    Text {
        pattern: String,
        fuzzy: Option<f32>,
    },
    Attr {
        key: String,
        value: String,
    },
    Backend(u64),
    Geometry {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    Combo(Vec<SelectorOrHint>),
}

#[derive(Clone, Debug)]
pub struct ResolveOpt {
    pub max_candidates: usize,
    pub fuzziness: Option<f32>,
    pub debounce_ms: Option<u64>,
}

impl Default for ResolveOpt {
    fn default() -> Self {
        Self {
            max_candidates: 1,
            fuzziness: None,
            debounce_ms: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Scope {
    Frame(FrameId),
    Page(PageId),
}

impl Default for Scope {
    fn default() -> Self {
        Scope::Page(PageId::new())
    }
}

#[derive(Clone, Debug, Copy)]
pub enum SnapLevel {
    Light,
    Full,
}

impl Default for SnapLevel {
    fn default() -> Self {
        SnapLevel::Full
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SnapshotId(pub String);

impl SnapshotId {
    pub fn new() -> Self {
        SnapshotId(Uuid::new_v4().to_string())
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

#[derive(Clone, Debug, Default)]
pub struct ScoreComponent {
    pub label: String,
    pub weight: f32,
    pub contribution: f32,
}

#[derive(Clone, Debug, Default)]
pub struct ScoreBreakdown {
    pub total: f32,
    pub components: Vec<ScoreComponent>,
}

#[derive(Clone, Debug)]
pub struct AnchorResolution {
    pub primary: AnchorDescriptor,
    pub candidates: Vec<AnchorDescriptor>,
    pub reason: String,
    pub score: ScoreBreakdown,
}

#[derive(Clone, Debug)]
pub struct JudgeReport {
    pub ok: bool,
    pub reason: String,
    pub facts: Value,
}

#[derive(Clone, Debug)]
pub struct DomAxSnapshot {
    pub id: SnapshotId,
    pub captured_at: Instant,
    pub page: PageId,
    pub frame: FrameId,
    pub session: Option<SessionId>,
    pub level: SnapLevel,
    pub dom_raw: Value,
    pub ax_raw: Value,
}

impl DomAxSnapshot {
    pub fn new(
        page: PageId,
        frame: FrameId,
        session: Option<SessionId>,
        level: SnapLevel,
        dom_raw: Value,
        ax_raw: Value,
    ) -> Self {
        Self {
            id: SnapshotId::new(),
            captured_at: Instant::now(),
            page,
            frame,
            session,
            level,
            dom_raw,
            ax_raw,
        }
    }
}

#[derive(Clone, Debug)]
pub enum DiffFocus {
    BackendNode(u64),
    Geometry { x: i32, y: i32, w: i32, h: i32 },
}

#[derive(Clone, Debug)]
pub struct DomAxDiff {
    pub base: Option<SnapshotId>,
    pub current: Option<SnapshotId>,
    pub generated_at: SystemTime,
    pub focus: Option<DiffFocus>,
    pub changes: Vec<Value>,
}

impl DomAxDiff {
    pub fn empty() -> Self {
        Self {
            base: None,
            current: None,
            generated_at: SystemTime::now(),
            focus: None,
            changes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct InteractionAdvice {
    pub summary: String,
    pub details: Value,
}

#[derive(Clone, Debug)]
pub struct SampledPair {
    pub dom: Value,
    pub ax: Value,
}

impl From<&SelectorOrHint> for ResolveHint {
    fn from(value: &SelectorOrHint) -> Self {
        match value {
            SelectorOrHint::Css(sel) => ResolveHint::Css(sel.clone()),
            SelectorOrHint::Aria { role, name, .. } => ResolveHint::Aria {
                role: role.clone(),
                name: name.clone(),
            },
            SelectorOrHint::Ax { role, name, .. } => ResolveHint::Aria {
                role: role.clone(),
                name: name.clone(),
            },
            SelectorOrHint::Text { pattern, .. } => ResolveHint::Text {
                pattern: pattern.clone(),
            },
            SelectorOrHint::Attr { key, value } => ResolveHint::Text {
                pattern: format!("@{}={}", key, value),
            },
            SelectorOrHint::Backend(id) => ResolveHint::Backend(*id),
            SelectorOrHint::Geometry { x, y, w, h } => ResolveHint::Geometry {
                x: *x,
                y: *y,
                w: *w,
                h: *h,
            },
            SelectorOrHint::Combo(items) => items
                .first()
                .map(|item| ResolveHint::from(item))
                .unwrap_or_else(|| ResolveHint::Css(String::new())),
        }
    }
}

impl From<&ResolveHint> for SelectorOrHint {
    fn from(value: &ResolveHint) -> Self {
        match value {
            ResolveHint::Css(sel) => SelectorOrHint::Css(sel.clone()),
            ResolveHint::Aria { role, name } => SelectorOrHint::Aria {
                role: role.clone(),
                name: name.clone(),
                state: None,
            },
            ResolveHint::Text { pattern } => SelectorOrHint::Text {
                pattern: pattern.clone(),
                fuzzy: None,
            },
            ResolveHint::Backend(id) => SelectorOrHint::Backend(*id),
            ResolveHint::Geometry { x, y, w, h } => SelectorOrHint::Geometry {
                x: *x,
                y: *y,
                w: *w,
                h: *h,
            },
        }
    }
}

impl From<&ResolveOptions> for ResolveOpt {
    fn from(options: &ResolveOptions) -> Self {
        ResolveOpt {
            max_candidates: options.max_candidates,
            fuzziness: options.fuzziness,
            debounce_ms: options.debounce_ms,
        }
    }
}
