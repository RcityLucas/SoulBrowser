use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use soulbrowser_core_types::{ActionId, PageId, SessionId};
use uuid::Uuid;

/// Unique identifier for recipe versions.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct RecipeId(pub String);

impl RecipeId {
    pub fn new() -> Self {
        Self(format!("r-{}", Uuid::new_v4()))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct RecipeVersion(pub u32);

impl RecipeVersion {
    pub fn next(&self) -> Self {
        RecipeVersion(self.0 + 1)
    }

    pub fn first() -> Self {
        RecipeVersion(1)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct RecVersion {
    pub id: RecipeId,
    pub version: RecipeVersion,
}

impl RecVersion {
    pub fn new(id: RecipeId, version: RecipeVersion) -> Self {
        Self { id, version }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RecType {
    AnchorLocate,
    ClickStrategy,
    TypeStrategy,
    SelectStrategy,
    FlowMacro,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Scope {
    pub origin: String,
    pub path_pat: Option<String>,
    pub page_template_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Preconditions {
    pub ax_role: Option<String>,
    pub text_hint: Option<String>,
    pub css_hint: Option<String>,
    pub url_contains: Option<String>,
    pub dom_features_hash: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Strategy {
    pub locator_chain: Vec<String>,
    pub primitive: String,
    pub options: serde_json::Value,
    pub gate: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EvidenceRefs {
    pub struct_id: Option<String>,
    pub pix_ids: Vec<String>,
    pub event_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scores {
    pub quality: f32,
    pub safety: f32,
    pub freshness: f32,
    pub support_n: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Status {
    Draft,
    Candidate,
    Active,
    Quarantined,
    Retired,
}

impl Status {
    pub fn should_index(&self) -> bool {
        matches!(self, Status::Active | Status::Candidate)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recipe {
    pub id: RecipeId,
    pub version: RecipeVersion,
    pub parent: Option<RecVersion>,
    pub r#type: RecType,
    pub scope: Scope,
    pub pre: Preconditions,
    pub strategy: Strategy,
    pub evidence: EvidenceRefs,
    pub scores: Scores,
    pub status: Status,
    pub labels: HashMap<String, String>,
    pub updated_at: DateTime<Utc>,
}

impl Recipe {
    pub fn rec_version(&self) -> RecVersion {
        RecVersion {
            id: self.id.clone(),
            version: self.version,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VectorItem {
    pub id: String,
    pub embedding: Vec<f32>,
    pub recipe_ref: Option<RecVersion>,
    pub payload: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecQuery {
    pub origin: String,
    pub path: Option<String>,
    pub ax_role: Option<String>,
    pub text_hint: Option<String>,
    pub css_hint: Option<String>,
    pub intent: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecContext {
    pub origin: String,
    pub path: Option<String>,
    pub primitive: String,
    pub anchor_fingerprint: Option<String>,
    pub intent: Option<String>,
    pub struct_id: Option<String>,
    pub pix_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OutcomeReason {
    Pass,
    Fail { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordExperienceReq {
    pub action: ActionId,
    pub outcome: OutcomeReason,
    pub context: RecContext,
    pub used_recipe: Option<RecVersion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Suggestion {
    pub recipe: RecVersion,
    pub rank: f32,
    pub expected_gain: Option<f32>,
    pub risks: Vec<String>,
    pub preview: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportBundle {
    pub recipes: Vec<Recipe>,
    pub vectors: Vec<VectorItem>,
    #[serde(default)]
    pub metadata: ExportMetadata,
    #[serde(default)]
    pub graph: Option<GraphSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportMetadata {
    pub generated_at: DateTime<Utc>,
    #[serde(default)]
    pub include_vectors: bool,
    #[serde(default)]
    pub include_payload: bool,
    #[serde(default)]
    pub incremental_from: Option<DateTime<Utc>>,
}

impl Default for ExportMetadata {
    fn default() -> Self {
        Self {
            generated_at: Utc::now(),
            include_vectors: true,
            include_payload: true,
            incremental_from: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportOptions {
    #[serde(default = "ExportOptions::default_include_vectors")]
    pub include_vectors: bool,
    #[serde(default = "ExportOptions::default_include_payload")]
    pub include_payload: bool,
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
}

impl ExportOptions {
    const fn default_include_vectors() -> bool {
        true
    }

    const fn default_include_payload() -> bool {
        true
    }
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_vectors: true,
            include_payload: true,
            since: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HygieneReport {
    pub merged: usize,
    pub quarantined: usize,
    pub retired: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphRelation {
    pub from: String,
    pub to: String,
    pub rel: String,
    pub weight: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphSnapshot {
    pub nodes: Vec<String>,
    pub edges: Vec<GraphRelation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecipesStatus {
    pub enabled: bool,
    pub ann_engine: String,
    pub recipe_total: usize,
    pub vector_total: usize,
    pub graph_edges: usize,
    #[serde(default)]
    pub last_hygiene: Option<DateTime<Utc>>,
}

/// Simple palette for ingest pipeline events.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestEvent {
    pub action: ActionId,
    pub session: Option<SessionId>,
    pub page: Option<PageId>,
    pub tags: SmallVec<[(String, String); 8]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecipeSnapshot {
    pub recipes: Vec<Recipe>,
}

impl RecipeSnapshot {
    pub fn new(mut recipes: Vec<Recipe>) -> Self {
        recipes.sort_by(|a, b| (a.id.0.clone(), a.version.0).cmp(&(b.id.0.clone(), b.version.0)));
        Self { recipes }
    }
}
