use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use parking_lot::{Mutex, MutexGuard};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::codec::{
    embed::{Embedder, RuleEmbedder},
    features,
};
use crate::errors::{RecError, RecErrorKind};
use crate::graph::store::{GraphEdge, GraphStore};
use crate::hygiene;
use crate::metrics::RecMetrics;
use crate::model::{
    ExportBundle, ExportMetadata, ExportOptions, OutcomeReason, RecContext, RecQuery, RecType,
    RecVersion, Recipe, RecipeId, RecipeSnapshot, RecipesStatus, RecordExperienceReq, Scores,
    Status, Suggestion, VectorItem,
};
use crate::policy::{AnnBackend, RecPolicyView, RollMode};
use crate::score;
use crate::storage::RecipeStore;
use crate::vector::ann::{AnnIndex, InMemoryAnn};
use crate::vector::hnsw_stub::HnswStub;

pub type RecResult<T> = Result<T, RecError>;

#[async_trait]
pub trait Recipes: Send + Sync {
    async fn record_experience(&self, req: RecordExperienceReq) -> RecResult<()>;
    async fn suggest_recipe(&self, query: RecQuery, top_k: usize) -> RecResult<Vec<Suggestion>>;
    async fn promote(&self, rec: RecVersion) -> RecResult<()>;
    async fn rollback(&self, rec: RecVersion) -> RecResult<()>;
    async fn quarantine(&self, rec: RecVersion, reason: String) -> RecResult<()>;
    async fn retire(&self, rec: RecVersion) -> RecResult<()>;
    async fn get_recipe(&self, rec: RecVersion) -> RecResult<Recipe>;
    async fn list_active(&self, origin: String) -> RecResult<Vec<Recipe>>;
    async fn export_index(&self) -> RecResult<ExportBundle>;
    async fn export_index_with_opts(&self, opts: ExportOptions) -> RecResult<ExportBundle>;
    async fn save_to_file(&self, path: &Path) -> RecResult<()>;
    async fn load_from_file(&self, path: &Path) -> RecResult<()>;
    async fn reload_policy(&self, policy: RecPolicyView) -> RecResult<()>;
    async fn status(&self) -> RecResult<RecipesStatus>;
}

#[derive(Clone)]
pub struct RecipesBuilder {
    policy: RecPolicyView,
    metrics: RecMetrics,
}

impl RecipesBuilder {
    pub fn new(policy: RecPolicyView) -> Self {
        Self {
            policy,
            metrics: RecMetrics::default(),
        }
    }

    pub fn with_metrics(mut self, metrics: RecMetrics) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn build(self) -> Arc<dyn Recipes> {
        Arc::new(InMemoryRecipes::new(self.policy, self.metrics))
    }
}

pub struct InMemoryRecipes {
    policy: Mutex<RecPolicyView>,
    metrics: RecMetrics,
    store: RecipeStore,
    ann: Mutex<Box<dyn AnnIndex + Send + Sync>>,
    embedder: Mutex<RuleEmbedder>,
    recipe_index: DashMap<String, RecipeId>,
    vector_index: DashMap<RecipeId, String>,
    graph: GraphStore,
    last_hygiene_run: Mutex<Option<DateTime<Utc>>>,
    snapshot_path: Mutex<PathBuf>,
    auto_persist: AtomicBool,
}

impl InMemoryRecipes {
    pub fn new(policy: RecPolicyView, metrics: RecMetrics) -> Self {
        let dim = policy.embed.dim.max(16);
        let ann = Self::init_ann(policy.embed.ann_engine.clone(), dim);
        Self {
            snapshot_path: Mutex::new(policy.io.snapshot_path.clone()),
            auto_persist: AtomicBool::new(policy.io.auto_persist),
            policy: Mutex::new(policy),
            metrics,
            store: RecipeStore::default(),
            ann: Mutex::new(ann),
            embedder: Mutex::new(RuleEmbedder::new(dim)),
            recipe_index: DashMap::new(),
            vector_index: DashMap::new(),
            graph: GraphStore::default(),
            last_hygiene_run: Mutex::new(None),
        }
    }

    fn init_ann(engine: AnnBackend, dim: usize) -> Box<dyn AnnIndex + Send + Sync> {
        match engine {
            AnnBackend::InMemory => Box::new(InMemoryAnn::new(dim)),
            AnnBackend::HnswStub => Box::new(HnswStub::new(dim)),
        }
    }

    fn policy(&self) -> MutexGuard<'_, RecPolicyView> {
        self.policy.lock()
    }

    fn embed_dim(&self) -> usize {
        self.embedder.lock().dim()
    }

    fn ensure_enabled(&self) -> RecResult<()> {
        if !self.policy().enabled {
            return Err(RecErrorKind::Disabled.into());
        }
        Ok(())
    }

    fn context_key(ctx: &RecContext) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            ctx.origin,
            ctx.path.as_deref().unwrap_or(""),
            ctx.primitive,
            ctx.intent.as_deref().unwrap_or(""),
            ctx.anchor_fingerprint.as_deref().unwrap_or("")
        )
    }

    fn privacy_allows(&self, query: &RecQuery) -> bool {
        if let Some(path) = &query.path {
            let forbid = {
                let policy = self.policy();
                policy.privacy.forbid_paths.clone()
            };
            if forbid.iter().any(|pattern| path.contains(pattern)) {
                return false;
            }
        }
        true
    }

    fn recipe_context_key(recipe: &Recipe) -> String {
        let anchor = recipe
            .strategy
            .locator_chain
            .first()
            .cloned()
            .unwrap_or_default();
        let intent = recipe.labels.get("intent").cloned().unwrap_or_default();
        format!(
            "{}|{}|{}|{}|{}",
            recipe.scope.origin,
            recipe.scope.path_pat.as_deref().unwrap_or(""),
            recipe.strategy.primitive,
            intent,
            anchor
        )
    }

    fn update_context_map(&self, recipe: &Recipe) -> Option<RecipeId> {
        let key = Self::recipe_context_key(recipe);
        if recipe.status.should_index() {
            self.recipe_index.insert(key, recipe.id.clone())
        } else {
            self.recipe_index.remove(&key).map(|(_, id)| id)
        }
    }

    fn add_graph_edge(&self, from: &RecipeId, to: &RecipeId, relation: &str, weight: f32) {
        if from == to {
            return;
        }
        let max_edges = {
            let policy = self.policy();
            policy.caps.max_edges_per_site
        };
        if self.graph.edges.len() >= max_edges {
            return;
        }
        if self
            .graph
            .edges
            .contains_key(&(from.0.clone(), to.0.clone()))
        {
            return;
        }
        self.graph.add_edge(
            from.0.clone(),
            to.0.clone(),
            GraphEdge {
                relation: relation.into(),
                weight,
            },
        );
    }

    fn register_graph_relations(&self, recipe: &Recipe, previous: Option<RecipeId>) {
        if let Some(parent) = &recipe.parent {
            self.add_graph_edge(&parent.id, &recipe.id, "supersedes", 1.0);
        }
        if let Some(prev) = previous {
            if prev != recipe.id {
                self.add_graph_edge(&prev, &recipe.id, "near", 0.8);
                self.add_graph_edge(&recipe.id, &prev, "near", 0.8);
            }
        }
    }

    fn rebuild_graph(&self) {
        self.graph.edges.clear();
        for recipe in self.store.latest_all() {
            if let Some(parent) = &recipe.parent {
                self.add_graph_edge(&parent.id, &recipe.id, "supersedes", 1.0);
            }
        }
    }
    fn store_recipe(&self, mut recipe: Recipe) -> Recipe {
        let next_version = self.store.next_version(&recipe.id);
        recipe.version = next_version;
        recipe.updated_at = Utc::now();
        self.store.insert(recipe.clone());
        self.update_vector_index(&recipe);
        let previous = self.update_context_map(&recipe);
        self.register_graph_relations(&recipe, previous);
        self.maybe_run_hygiene();
        recipe
    }

    fn rebuild_indexes(&self) {
        self.recipe_index.clear();
        self.vector_index.clear();
        let ann_engine = { self.policy().embed.ann_engine.clone() };
        let dim = self.embed_dim();
        {
            let mut ann = self.ann.lock();
            *ann = Self::init_ann(ann_engine, dim);
        }
        self.graph.edges.clear();
        for recipe in self.store.latest_all() {
            self.update_vector_index(&recipe);
            let previous = self.update_context_map(&recipe);
            self.register_graph_relations(&recipe, previous);
        }
    }

    fn persist_snapshot(&self) -> RecResult<()> {
        let snapshot = RecipeSnapshot::new(self.store.all_recipes());
        let path = { self.snapshot_path.lock().clone() };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| RecErrorKind::Internal(err.to_string()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .map_err(|err| RecErrorKind::Internal(err.to_string()))?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, &snapshot)
            .map_err(|err| RecErrorKind::Internal(err.to_string()).into())
    }

    fn load_snapshot_from_path(&self, path: &Path) -> RecResult<()> {
        let file = File::open(path).map_err(|err| RecErrorKind::Internal(err.to_string()))?;
        let reader = BufReader::new(file);
        let snapshot: RecipeSnapshot = serde_json::from_reader(reader)
            .map_err(|err| RecErrorKind::Internal(err.to_string()))?;
        self.store.replace_all(snapshot.recipes);
        self.rebuild_indexes();
        Ok(())
    }

    fn maybe_autosave(&self) {
        if !self.auto_persist.load(Ordering::Relaxed) {
            return;
        }
        if let Err(err) = self.persist_snapshot() {
            eprintln!("[recipes] auto snapshot failed: {}", err);
        }
    }

    fn update_vector_index(&self, recipe: &Recipe) {
        if !self.should_index(recipe) {
            self.remove_vector(&recipe.id);
            return;
        }
        let features = features::from_recipe(recipe);
        let embedding = {
            let embedder = self.embedder.lock();
            embedder.encode(&features)
        };
        let mut payload = HashMap::new();
        payload.insert("origin".into(), recipe.scope.origin.clone());
        if let Some(path) = &recipe.scope.path_pat {
            payload.insert("path".into(), path.clone());
        }
        payload.insert("status".into(), format!("{:?}", recipe.status));
        let vector_id = Self::vector_id(recipe);
        let mut ann = self.ann.lock();
        if let Some(prev) = self
            .vector_index
            .insert(recipe.id.clone(), vector_id.clone())
        {
            ann.remove(&prev);
        }
        ann.insert(VectorItem {
            id: vector_id,
            embedding,
            recipe_ref: Some(recipe.rec_version()),
            payload,
        });
    }

    fn remove_vector(&self, recipe_id: &RecipeId) {
        if let Some((_, previous)) = self.vector_index.remove(recipe_id) {
            let mut ann = self.ann.lock();
            ann.remove(&previous);
        }
    }

    fn should_index(&self, recipe: &Recipe) -> bool {
        recipe.status.should_index()
    }

    fn vector_id(recipe: &Recipe) -> String {
        format!("vec-{}-{}", recipe.id.0, recipe.version.0)
    }

    fn evaluate_status(&self, previous: Status, scores: &Scores) -> Status {
        let thresholds = {
            let policy = self.policy();
            policy.thresholds.clone()
        };
        if scores.safety < thresholds.activate_safety * 0.5 {
            Status::Quarantined
        } else if scores.quality >= thresholds.activate_quality
            && scores.safety >= thresholds.activate_safety
            && (scores.support_n as usize) >= thresholds.min_support_n
        {
            Status::Active
        } else if matches!(previous, Status::Retired | Status::Quarantined) {
            previous
        } else if scores.quality < thresholds.suggest * 0.5 {
            Status::Draft
        } else {
            Status::Candidate
        }
    }

    fn build_new_recipe(&self, ctx: &RecContext) -> Recipe {
        let mut labels = HashMap::new();
        if let Some(intent) = &ctx.intent {
            labels.insert("intent".into(), intent.clone());
        }
        let evidence = crate::model::EvidenceRefs {
            struct_id: ctx.struct_id.clone(),
            pix_ids: ctx.pix_ids.clone(),
            event_ids: Vec::new(),
        };
        let locator = ctx
            .anchor_fingerprint
            .clone()
            .unwrap_or_else(|| ctx.primitive.clone());
        let pre = crate::model::Preconditions {
            ax_role: None,
            text_hint: ctx.intent.clone(),
            css_hint: ctx.anchor_fingerprint.clone(),
            url_contains: ctx.path.clone(),
            dom_features_hash: None,
        };
        let strategy = crate::model::Strategy {
            locator_chain: vec![locator],
            primitive: ctx.primitive.clone(),
            options: json!({}),
            gate: json!({}),
        };
        Recipe {
            id: RecipeId::new(),
            version: crate::model::RecipeVersion::first(),
            parent: None,
            r#type: RecType::AnchorLocate,
            scope: crate::model::Scope {
                origin: ctx.origin.clone(),
                path_pat: ctx.path.clone(),
                page_template_id: None,
            },
            pre,
            strategy,
            evidence,
            scores: score::bootstrap(&OutcomeReason::Pass),
            status: Status::Candidate,
            labels,
            updated_at: Utc::now(),
        }
    }

    fn update_existing(&self, base: Recipe, ctx: &RecContext, outcome: &OutcomeReason) -> Recipe {
        let mut next = base.clone();
        next.parent = Some(base.rec_version());
        let freshness = {
            let policy = self.policy();
            policy.freshness_tau_sec
        };
        next.scores = score::apply_outcome(&base.scores, outcome, freshness);
        if next.evidence.struct_id.is_none() {
            next.evidence.struct_id = ctx.struct_id.clone();
        }
        for pix in &ctx.pix_ids {
            if !next.evidence.pix_ids.contains(pix) {
                next.evidence.pix_ids.push(pix.clone());
            }
        }
        if let Some(intent) = &ctx.intent {
            next.labels
                .entry("intent".into())
                .or_insert_with(|| intent.clone());
        }
        next.status = self.evaluate_status(base.status, &next.scores);
        self.store_recipe(next)
    }

    fn maybe_run_hygiene(&self) {
        let schedule = {
            let policy = self.policy();
            policy.hygiene.schedule_min_sec
        };
        if schedule == 0 {
            return;
        }
        let mut guard = self.last_hygiene_run.lock();
        let now = Utc::now();
        let should_run = guard.map_or(true, |ts| {
            now - ts >= chrono::Duration::seconds(schedule as i64)
        });
        if should_run {
            let snapshot = self.store.latest_all();
            let _report = hygiene::run_hygiene(&snapshot);
            self.metrics.record_hygiene();
            *guard = Some(now);
        }
    }

    fn handle_pass(&self, req: &RecordExperienceReq) -> RecResult<()> {
        let key = Self::context_key(&req.context);
        let target_id = req
            .used_recipe
            .as_ref()
            .map(|ver| ver.id.clone())
            .or_else(|| self.recipe_index.get(&key).map(|entry| entry.clone()));

        if let Some(recipe_id) = target_id {
            if let Some(latest) = self.store.latest(&recipe_id) {
                self.update_existing(latest, &req.context, &req.outcome);
                self.maybe_autosave();
                return Ok(());
            }
        }

        let recipe = self.build_new_recipe(&req.context);
        self.store_recipe(recipe);
        self.maybe_autosave();
        Ok(())
    }

    fn handle_fail(&self, req: &RecordExperienceReq) -> RecResult<()> {
        let key = Self::context_key(&req.context);
        let target_id = req
            .used_recipe
            .as_ref()
            .map(|ver| ver.id.clone())
            .or_else(|| self.recipe_index.get(&key).map(|entry| entry.clone()));

        if let Some(recipe_id) = target_id {
            if let Some(latest) = self.store.latest(&recipe_id) {
                self.update_existing(latest, &req.context, &req.outcome);
                self.maybe_autosave();
            }
        }
        Ok(())
    }

    fn run_transition(&self, ver: RecVersion, target: Status) -> RecResult<()> {
        let recipe = self
            .store
            .find_version(&ver)
            .ok_or(RecErrorKind::NotFound)?;
        let mut next = recipe.clone();
        next.parent = Some(ver.clone());
        next.status = target;
        self.store_recipe(next);
        self.maybe_autosave();
        Ok(())
    }
}

#[async_trait]
impl Recipes for InMemoryRecipes {
    async fn record_experience(&self, req: RecordExperienceReq) -> RecResult<()> {
        self.ensure_enabled()?;
        self.metrics.record_ingest();
        match req.outcome {
            OutcomeReason::Pass => self.handle_pass(&req)?,
            OutcomeReason::Fail { .. } => self.handle_fail(&req)?,
        }
        Ok(())
    }

    async fn suggest_recipe(&self, query: RecQuery, top_k: usize) -> RecResult<Vec<Suggestion>> {
        self.ensure_enabled()?;
        if !self.privacy_allows(&query) {
            return Ok(Vec::new());
        }
        let features = features::from_query(&query);
        let embedding = {
            let embedder = self.embedder.lock();
            embedder.encode(&features)
        };
        let (thresholds, high_risk_intents) = {
            let policy = self.policy();
            (
                policy.thresholds.clone(),
                policy.privacy.high_risk_intents.clone(),
            )
        };
        let ann = self.ann.lock();
        let hits = ann.search(&embedding, top_k.saturating_mul(3).max(1));
        drop(ann);

        let mut suggestions = Vec::new();
        for (item, sim) in hits {
            let Some(recipe_ref) = &item.recipe_ref else {
                continue;
            };
            let recipe = self
                .store
                .find_version(recipe_ref)
                .or_else(|| self.store.latest(&recipe_ref.id));
            let Some(recipe) = recipe else {
                continue;
            };
            if let Some(origin) = item.payload.get("origin") {
                if origin != &query.origin {
                    continue;
                }
            }
            if !recipe.status.should_index() {
                continue;
            }
            let rank = 0.5 * sim + 0.3 * recipe.scores.quality + 0.2 * recipe.scores.freshness;
            if rank < thresholds.suggest {
                continue;
            }
            if let Some(intent) = &query.intent {
                if high_risk_intents.iter().any(|risk| intent.contains(risk))
                    && recipe.scores.safety < thresholds.activate_safety
                {
                    continue;
                }
            }
            let mut risks = Vec::new();
            if recipe.scores.safety < thresholds.activate_safety {
                risks.push("low_safety".into());
            }
            suggestions.push(Suggestion {
                recipe: recipe.rec_version(),
                rank,
                expected_gain: Some(recipe.scores.quality),
                risks,
                preview: json!({
                    "strategy": recipe.strategy.locator_chain,
                    "primitive": recipe.strategy.primitive,
                }),
            });
            if suggestions.len() == top_k {
                break;
            }
        }
        suggestions.sort_by(|a, b| {
            b.rank
                .partial_cmp(&a.rank)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(suggestions)
    }

    async fn promote(&self, rec: RecVersion) -> RecResult<()> {
        self.ensure_enabled()?;
        let recipe = self
            .store
            .find_version(&rec)
            .ok_or(RecErrorKind::NotFound)?;
        let thresholds = {
            let policy = self.policy();
            policy.thresholds.clone()
        };
        if recipe.scores.quality < thresholds.activate_quality
            || recipe.scores.safety < thresholds.activate_safety
        {
            return Err(
                RecErrorKind::InvalidInput("recipe below activation threshold".into()).into(),
            );
        }
        let rollout_mode = { self.policy().rollout.default.clone() };
        let target = match rollout_mode {
            RollMode::Canary | RollMode::Immediate => Status::Active,
            RollMode::Manual => Status::Candidate,
        };
        self.run_transition(rec.clone(), target)?;
        self.metrics.record_governance("promote");
        Ok(())
    }

    async fn rollback(&self, rec: RecVersion) -> RecResult<()> {
        self.ensure_enabled()?;
        let recipe = self
            .store
            .find_version(&rec)
            .ok_or(RecErrorKind::NotFound)?;
        let parent = recipe
            .parent
            .clone()
            .ok_or_else(|| RecErrorKind::InvalidInput("no parent to rollback".into()))?;
        let parent_recipe = self
            .store
            .find_version(&parent)
            .ok_or(RecErrorKind::NotFound)?;
        let mut restored = parent_recipe.clone();
        restored.parent = Some(rec);
        self.store_recipe(restored);
        self.maybe_autosave();
        self.metrics.record_governance("rollback");
        Ok(())
    }

    async fn quarantine(&self, rec: RecVersion, _reason: String) -> RecResult<()> {
        self.ensure_enabled()?;
        self.run_transition(rec.clone(), Status::Quarantined)?;
        self.metrics.record_governance("quarantine");
        Ok(())
    }

    async fn retire(&self, rec: RecVersion) -> RecResult<()> {
        self.ensure_enabled()?;
        self.run_transition(rec.clone(), Status::Retired)?;
        self.metrics.record_governance("retire");
        Ok(())
    }

    async fn get_recipe(&self, rec: RecVersion) -> RecResult<Recipe> {
        self.store
            .find_version(&rec)
            .ok_or(RecErrorKind::NotFound.into())
    }

    async fn list_active(&self, origin: String) -> RecResult<Vec<Recipe>> {
        let recipes = self
            .store
            .latest_all()
            .into_iter()
            .filter(|r| r.scope.origin == origin && matches!(r.status, Status::Active))
            .collect();
        Ok(recipes)
    }

    async fn export_index(&self) -> RecResult<ExportBundle> {
        self.export_index_with_opts(ExportOptions::default()).await
    }

    async fn export_index_with_opts(&self, opts: ExportOptions) -> RecResult<ExportBundle> {
        self.ensure_enabled()?;
        let now = Utc::now();
        let mut recipes = self.store.latest_all();
        if let Some(since) = opts.since {
            recipes.retain(|recipe| recipe.updated_at >= since);
        }

        let include_payload = opts.include_payload;
        let recipes: Vec<Recipe> = recipes
            .into_iter()
            .map(|mut recipe| {
                if !include_payload {
                    recipe.evidence.event_ids.clear();
                    recipe.evidence.pix_ids.clear();
                    recipe.strategy.options = json!({});
                    recipe.strategy.gate = json!({});
                }
                recipe
            })
            .collect();

        let allowed_ids: HashSet<String> = recipes.iter().map(|r| r.id.0.clone()).collect();

        let vectors = if opts.include_vectors {
            let mut items = self.ann.lock().items();
            if opts.since.is_some() || !allowed_ids.is_empty() {
                items.retain(|item| {
                    item.recipe_ref
                        .as_ref()
                        .map(|rec| allowed_ids.contains(&rec.id.0))
                        .unwrap_or(false)
                });
            }
            if !include_payload {
                for item in &mut items {
                    item.payload.clear();
                }
            }
            items
        } else {
            Vec::new()
        };

        let metadata = ExportMetadata {
            generated_at: now,
            include_vectors: opts.include_vectors,
            include_payload,
            incremental_from: opts.since,
        };

        let graph = if recipes.is_empty() {
            None
        } else {
            let mut snapshot = self.graph.snapshot(Some(&allowed_ids));
            for id in &allowed_ids {
                if !snapshot.nodes.iter().any(|node| node == id) {
                    snapshot.nodes.push(id.clone());
                }
            }
            Some(snapshot)
        };

        Ok(ExportBundle {
            recipes,
            vectors,
            metadata,
            graph,
        })
    }

    async fn save_to_file(&self, path: &Path) -> RecResult<()> {
        self.ensure_enabled()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| RecErrorKind::Internal(err.to_string()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .map_err(|err| RecErrorKind::Internal(err.to_string()))?;
        let writer = BufWriter::new(file);
        let snapshot = RecipeSnapshot {
            recipes: self.store.all_recipes(),
        };
        serde_json::to_writer_pretty(writer, &snapshot)
            .map_err(|err| RecErrorKind::Internal(err.to_string()))?;
        Ok(())
    }

    async fn load_from_file(&self, path: &Path) -> RecResult<()> {
        self.ensure_enabled()?;
        let file = File::open(path).map_err(|err| RecErrorKind::Internal(err.to_string()))?;
        let reader = BufReader::new(file);
        let snapshot: RecipeSnapshot = serde_json::from_reader(reader)
            .map_err(|err| RecErrorKind::Internal(err.to_string()))?;
        self.store.replace_all(snapshot.recipes.clone());
        self.rebuild_indexes();
        Ok(())
    }

    async fn reload_policy(&self, policy: RecPolicyView) -> RecResult<()> {
        let mut current = self.policy();
        let old = current.clone();
        let dim_changed = old.embed.dim != policy.embed.dim;
        let ann_changed = old.embed.ann_engine != policy.embed.ann_engine;
        let snapshot_changed = old.io.snapshot_path != policy.io.snapshot_path;
        let auto_changed = old.io.auto_persist != policy.io.auto_persist;
        *current = policy.clone();
        drop(current);

        if dim_changed {
            let mut embedder = self.embedder.lock();
            *embedder = RuleEmbedder::new(policy.embed.dim.max(16));
        }

        if dim_changed || ann_changed {
            let new_ann = Self::init_ann(policy.embed.ann_engine.clone(), policy.embed.dim.max(16));
            {
                let mut ann = self.ann.lock();
                *ann = new_ann;
            }
            self.rebuild_indexes();
        }

        if snapshot_changed {
            *self.snapshot_path.lock() = policy.io.snapshot_path.clone();
        }
        if auto_changed {
            self.auto_persist
                .store(policy.io.auto_persist, Ordering::Relaxed);
        }

        let max_edges = policy.caps.max_edges_per_site;
        if self.graph.edges.len() > max_edges {
            let mut keys: Vec<_> = self
                .graph
                .edges
                .iter()
                .map(|entry| entry.key().clone())
                .collect();
            keys.sort();
            for key in keys.into_iter().skip(max_edges) {
                self.graph.edges.remove(&key);
            }
        }

        self.metrics.record_policy_reload();
        Ok(())
    }

    async fn status(&self) -> RecResult<RecipesStatus> {
        let (enabled, ann_engine) = {
            let policy = self.policy();
            (policy.enabled, format!("{:?}", policy.embed.ann_engine))
        };
        let recipe_total = self.store.latest_all().len();
        let vector_total = self.vector_index.len();
        let graph_edges = self.graph.edges.len();
        let last_hygiene = *self.last_hygiene_run.lock();
        Ok(RecipesStatus {
            enabled,
            ann_engine,
            recipe_total,
            vector_total,
            graph_edges,
            last_hygiene,
        })
    }
}
