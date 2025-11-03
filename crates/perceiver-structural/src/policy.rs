use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use soulbrowser_policy_center::model::{
    StructuralCachePolicy, StructuralDiffFocus, StructuralDiffGeometry, StructuralDiffPolicy,
    StructuralJudgePolicy, StructuralScoreWeights,
};
use soulbrowser_policy_center::{
    PolicyCenter, PolicySnapshot, StructuralPerceiverPolicy, StructuralResolvePolicy,
};

use crate::DiffFocus;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ResolveOptions {
    pub max_candidates: usize,
    pub fuzziness: Option<f32>,
    pub debounce_ms: Option<u64>,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        Self {
            max_candidates: 0,
            fuzziness: None,
            debounce_ms: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreWeights {
    pub visibility: f32,
    pub accessibility: f32,
    pub text: f32,
    pub geometry: f32,
    pub backend: f32,
}

impl ScoreWeights {
    pub fn normalize(&mut self) {
        if self.visibility < 0.0 {
            self.visibility = 0.0;
        }
        if self.accessibility < 0.0 {
            self.accessibility = 0.0;
        }
        if self.text < 0.0 {
            self.text = 0.0;
        }
        if self.geometry < 0.0 {
            self.geometry = 0.0;
        }
        if self.backend < 0.0 {
            self.backend = 0.0;
        }
    }
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            visibility: 0.05,
            accessibility: 0.06,
            text: 0.05,
            geometry: 0.1,
            backend: 0.25,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JudgePolicy {
    pub minimum_opacity: Option<f32>,
    pub minimum_visible_area: Option<f64>,
    pub pointer_events_block: bool,
}

impl Default for JudgePolicy {
    fn default() -> Self {
        Self {
            minimum_opacity: None,
            minimum_visible_area: None,
            pointer_events_block: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct DiffPolicy {
    pub debounce_ms: Option<u64>,
    pub max_changes: Option<usize>,
    pub focus: Option<DiffPolicyFocus>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct DiffPolicyFocus {
    pub backend_node_id: Option<u64>,
    pub geometry: Option<DiffFocusGeometry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct DiffFocusGeometry {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachePolicy {
    pub anchor_ttl_ms: u64,
    pub snapshot_ttl_ms: u64,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            anchor_ttl_ms: 250,
            snapshot_ttl_ms: 1_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PerceiverPolicyView {
    pub resolve: ResolveOptions,
    pub weights: ScoreWeights,
    pub judge: JudgePolicy,
    pub diff: DiffPolicy,
    pub cache: CachePolicy,
}

impl PerceiverPolicyView {
    pub fn normalize(&mut self) {
        self.weights.normalize();
    }

    pub async fn load_from_center(center: Arc<dyn PolicyCenter + Send + Sync>) -> Self {
        let snapshot = center.snapshot().await;
        let mut view = Self::from_snapshot(&snapshot);
        view.normalize();
        view
    }

    pub fn from_snapshot(snapshot: &PolicySnapshot) -> Self {
        Self::from_structural(&snapshot.perceiver.structural)
    }

    pub fn from_structural(policy: &StructuralPerceiverPolicy) -> Self {
        Self {
            resolve: ResolveOptions::from(&policy.resolve),
            weights: ScoreWeights::from_structural(&policy.weights),
            judge: JudgePolicy::from_structural(&policy.judge),
            diff: DiffPolicy::from_structural(&policy.diff),
            cache: CachePolicy::from_structural(&policy.cache),
        }
    }
}

impl Default for PerceiverPolicyView {
    fn default() -> Self {
        let structural = StructuralPerceiverPolicy::default();
        let mut view = Self::from_structural(&structural);
        view.normalize();
        view
    }
}

impl From<&StructuralResolvePolicy> for ResolveOptions {
    fn from(policy: &StructuralResolvePolicy) -> Self {
        Self {
            max_candidates: policy.max_candidates,
            fuzziness: policy.fuzziness,
            debounce_ms: policy.debounce_ms,
        }
    }
}

impl ScoreWeights {
    fn from_structural(structural: &StructuralScoreWeights) -> Self {
        Self {
            visibility: structural.visibility,
            accessibility: structural.accessibility,
            text: structural.text,
            geometry: structural.geometry,
            backend: structural.backend,
        }
    }
}

impl JudgePolicy {
    fn from_structural(structural: &StructuralJudgePolicy) -> Self {
        Self {
            minimum_opacity: structural.minimum_opacity,
            minimum_visible_area: structural.minimum_visible_area,
            pointer_events_block: structural.pointer_events_block,
        }
    }
}

impl DiffPolicy {
    fn from_structural(structural: &StructuralDiffPolicy) -> Self {
        Self {
            debounce_ms: structural.debounce_ms,
            max_changes: structural.max_changes,
            focus: structural
                .focus
                .as_ref()
                .map(DiffPolicyFocus::from_structural),
        }
    }
}

impl DiffPolicyFocus {
    fn from_structural(structural: &StructuralDiffFocus) -> Self {
        Self {
            backend_node_id: structural.backend_node_id,
            geometry: structural
                .geometry
                .as_ref()
                .map(DiffFocusGeometry::from_structural),
        }
    }

    pub fn to_model_focus(&self) -> Option<DiffFocus> {
        if let Some(geom) = &self.geometry {
            return Some(DiffFocus::Geometry {
                x: geom.x,
                y: geom.y,
                w: geom.w,
                h: geom.h,
            });
        }
        self.backend_node_id.map(DiffFocus::BackendNode)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "backend_node_id": self.backend_node_id,
            "geometry": self.geometry.as_ref().map(DiffFocusGeometry::to_json),
        })
    }
}

impl DiffFocusGeometry {
    fn from_structural(structural: &StructuralDiffGeometry) -> Self {
        Self {
            x: structural.x,
            y: structural.y,
            w: structural.w,
            h: structural.h,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "x": self.x,
            "y": self.y,
            "w": self.w,
            "h": self.h,
        })
    }
}

impl CachePolicy {
    fn from_structural(structural: &StructuralCachePolicy) -> Self {
        Self {
            anchor_ttl_ms: structural.anchor_ttl_ms,
            snapshot_ttl_ms: structural.snapshot_ttl_ms,
        }
    }
}
