use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::RwLock;
use serde_json::{Map as JsonMap, Value};
use soulbrowser_core_types::ExecRoute;

use crate::api::StructuralPerceiver;
use crate::cache::{AnchorCache, SnapshotCache};
use crate::differ;
use crate::errors::PerceiverError;
use crate::events;
use crate::judges;
use crate::model::{
    AnchorDescriptor, AnchorGeometry, AnchorResolution, DomAxDiff, DomAxSnapshot, ResolveHint,
    ResolveOpt, SampledPair, Scope, ScoreBreakdown, SelectorOrHint, SnapLevel,
};
use crate::policy::{PerceiverPolicyView, ResolveOptions};
use crate::ports::CdpPerceptionPort;
use crate::resolver::{generate, rank};
use crate::sampler::Sampler;
use crate::{reason, redact};
use soulbrowser_core_types::PageId;
use soulbrowser_policy_center::PolicyCenter;
use soulbrowser_state_center::{PerceiverEvent, ScoreComponentRecord, StateCenter, StateEvent};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

pub struct StructuralPerceiverImpl<P>
where
    P: CdpPerceptionPort + Send + Sync,
{
    sampler: Sampler<P>,
    anchor_cache: Arc<AnchorCache>,
    snapshot_cache: Arc<SnapshotCache>,
    state_center: Option<Arc<dyn StateCenter>>,
    policy: Arc<RwLock<PerceiverPolicyView>>,
    policy_task: Option<JoinHandle<()>>,
}

impl<P> StructuralPerceiverImpl<P>
where
    P: CdpPerceptionPort + Send + Sync,
{
    pub fn new(port: Arc<P>) -> Self {
        Self::with_state_center_inner(port, None, PerceiverPolicyView::default())
    }

    pub fn with_state_center(port: Arc<P>, state_center: Arc<dyn StateCenter>) -> Self {
        Self::with_state_center_inner(port, Some(state_center), PerceiverPolicyView::default())
    }

    pub fn with_policy(port: Arc<P>, policy: PerceiverPolicyView) -> Self {
        Self::with_state_center_inner(port, None, policy)
    }

    pub async fn with_live_policy(
        port: Arc<P>,
        policy_center: Arc<dyn PolicyCenter + Send + Sync>,
    ) -> Self {
        let policy = PerceiverPolicyView::load_from_center(Arc::clone(&policy_center)).await;
        let mut instance = Self::with_state_center_inner(port, None, policy);
        instance.spawn_policy_watcher(policy_center);
        instance
    }

    pub async fn with_state_center_and_live_policy(
        port: Arc<P>,
        state_center: Arc<dyn StateCenter>,
        policy_center: Arc<dyn PolicyCenter + Send + Sync>,
    ) -> Self {
        let policy = PerceiverPolicyView::load_from_center(Arc::clone(&policy_center)).await;
        let mut instance = Self::with_state_center_inner(port, Some(state_center), policy);
        instance.spawn_policy_watcher(policy_center);
        instance
    }

    pub fn with_state_center_and_policy(
        port: Arc<P>,
        state_center: Arc<dyn StateCenter>,
        policy: PerceiverPolicyView,
    ) -> Self {
        Self::with_state_center_inner(port, Some(state_center), policy)
    }

    fn with_state_center_inner(
        port: Arc<P>,
        state_center: Option<Arc<dyn StateCenter>>,
        mut policy: PerceiverPolicyView,
    ) -> Self {
        policy.normalize();
        let anchor_ttl = Duration::from_millis(policy.cache.anchor_ttl_ms);
        let snapshot_ttl = Duration::from_millis(policy.cache.snapshot_ttl_ms);
        Self {
            sampler: Sampler::new(port),
            anchor_cache: Arc::new(AnchorCache::new(anchor_ttl)),
            snapshot_cache: Arc::new(SnapshotCache::new(snapshot_ttl)),
            state_center,
            policy: Arc::new(RwLock::new(policy)),
            policy_task: None,
        }
    }

    fn apply_policy_internal(
        policy_lock: &Arc<RwLock<PerceiverPolicyView>>,
        anchor_cache: &Arc<AnchorCache>,
        snapshot_cache: &Arc<SnapshotCache>,
        mut policy: PerceiverPolicyView,
    ) {
        policy.normalize();
        anchor_cache.set_ttl(Duration::from_millis(policy.cache.anchor_ttl_ms));
        snapshot_cache.set_ttl(Duration::from_millis(policy.cache.snapshot_ttl_ms));
        *policy_lock.write() = policy;
    }

    pub fn set_policy(&self, policy: PerceiverPolicyView) {
        Self::apply_policy_internal(
            &self.policy,
            &self.anchor_cache,
            &self.snapshot_cache,
            policy,
        );
    }

    pub fn policy_view(&self) -> PerceiverPolicyView {
        self.policy.read().clone()
    }

    pub fn invalidate_for_route(&self, route: &ExecRoute) {
        self.invalidate_for_page(&route.page);
    }

    pub fn invalidate_for_page(&self, page: &PageId) {
        let anchor_prefix = format!("{:?}::", page);
        self.anchor_cache.invalidate_prefix(&anchor_prefix);
        let snapshot_prefix = format!("snapshot:{}:", page.0);
        self.snapshot_cache.invalidate_prefix(&snapshot_prefix);
    }

    pub fn invalidate_all(&self) {
        self.anchor_cache.clear();
        self.snapshot_cache.clear();
    }

    /// Get reference to anchor cache (for lifecycle watcher integration).
    pub fn get_anchor_cache(&self) -> Arc<AnchorCache> {
        Arc::clone(&self.anchor_cache)
    }

    /// Get reference to snapshot cache (for lifecycle watcher integration).
    pub fn get_snapshot_cache(&self) -> Arc<SnapshotCache> {
        Arc::clone(&self.snapshot_cache)
    }

    fn selector_requires_ax(selector: &SelectorOrHint) -> bool {
        match selector {
            SelectorOrHint::Aria { .. } | SelectorOrHint::Ax { .. } => true,
            SelectorOrHint::Combo(items) => items.iter().any(Self::selector_requires_ax),
            _ => false,
        }
    }

    async fn sample_for_resolve(
        &self,
        route: &ExecRoute,
        scope: &Scope,
        selector: &SelectorOrHint,
    ) -> Option<SampledPair> {
        match self.sampler.sample(route, scope, SnapLevel::Light).await {
            Ok(light) => {
                if Self::selector_requires_ax(selector) {
                    match self.sampler.sample(route, scope, SnapLevel::Full).await {
                        Ok(full) => Some(full),
                        Err(err) => {
                            warn!(?err, "full snapshot failed after light snapshot; continuing with light snapshot");
                            Some(light)
                        }
                    }
                } else {
                    Some(light)
                }
            }
            Err(err) => {
                warn!(?err, "light snapshot failed; retrying with full snapshot");
                match self.sampler.sample(route, scope, SnapLevel::Full).await {
                    Ok(full) => Some(full),
                    Err(err_full) => {
                        warn!(
                            ?err_full,
                            "perceiver sampling failed for both light/full snapshots"
                        );
                        None
                    }
                }
            }
        }
    }

    fn spawn_policy_watcher(&mut self, center: Arc<dyn PolicyCenter + Send + Sync>) {
        if let Some(handle) = self.policy_task.take() {
            handle.abort();
        }
        let mut rx = center.subscribe();
        let policy_lock = Arc::clone(&self.policy);
        let anchor_cache = Arc::clone(&self.anchor_cache);
        let snapshot_cache = Arc::clone(&self.snapshot_cache);
        self.policy_task = Some(tokio::spawn(async move {
            loop {
                match rx.changed().await {
                    Ok(()) => {
                        let snapshot_arc = rx.borrow().clone();
                        let view = PerceiverPolicyView::from_snapshot(snapshot_arc.as_ref());
                        Self::apply_policy_internal(
                            &policy_lock,
                            &anchor_cache,
                            &snapshot_cache,
                            view,
                        );
                    }
                    Err(err) => {
                        debug!(?err, "policy watcher channel closed");
                        break;
                    }
                }
            }
        }));
    }

    async fn ensure_anchor_metadata(
        &self,
        route: &ExecRoute,
        anchor: &mut AnchorDescriptor,
    ) -> Result<(), PerceiverError> {
        let object = anchor.value.as_object();
        let has_attrs = object.and_then(|obj| obj.get("attributes")).is_some();
        let has_computed = object.and_then(|obj| obj.get("computedStyle")).is_some();
        if has_attrs && has_computed {
            return Ok(());
        }
        let backend_node_id = match anchor.backend_node_id {
            Some(id) => id,
            None => return Ok(()),
        };

        let mut map = object.cloned().unwrap_or_default();

        if !has_attrs {
            if let Some(value) = self.sampler.node_attributes(route, backend_node_id).await? {
                if let Some(attrs) = value.as_object() {
                    map.insert("attributes".into(), Value::Object(attrs.clone()));
                }
            }
        }

        if !has_computed {
            if let Some(value) = self.sampler.node_style(route, backend_node_id).await? {
                if let Some(style) = value.as_object() {
                    map.insert("computedStyle".into(), Value::Object(style.clone()));
                } else {
                    map.insert("computedStyle".into(), value);
                }
            }
        }

        if map.is_empty() && object.is_none() {
            return Ok(());
        }

        anchor.value = Value::Object(map);
        Ok(())
    }

    async fn record_state_event(&self, event: PerceiverEvent) {
        if let Some(center) = self.state_center.as_ref() {
            if let Err(err) = center.append(StateEvent::perceiver(event)).await {
                warn!(?err, "failed to append perceiver event to state center");
            }
        }
    }

    async fn resolve_with_selector(
        &self,
        route: ExecRoute,
        selector: SelectorOrHint,
        mut options: ResolveOpt,
        legacy_hint: Option<ResolveHint>,
        policy: &PerceiverPolicyView,
    ) -> Result<AnchorResolution, PerceiverError> {
        let timer_start = Instant::now();
        if options.max_candidates == 0 {
            options.max_candidates = policy.resolve.max_candidates;
        }
        if options.max_candidates == 0 {
            options.max_candidates = 1;
        }
        if options.fuzziness.is_none() {
            options.fuzziness = policy.resolve.fuzziness;
        }
        if options.debounce_ms.is_none() {
            options.debounce_ms = policy.resolve.debounce_ms;
        }

        let cache_key = format!("{:?}::{}", route.page, selector.cache_key());
        let debounce_override = options.debounce_ms.map(|ms| Duration::from_millis(ms));
        if let Some(hit) = self.anchor_cache.get(&cache_key, debounce_override) {
            let (cached_score, cached_reason) = if hit.score.components.is_empty() {
                let fallback = reason::from_confidence(hit.primary.confidence);
                let summary = reason::summarize(&fallback);
                (fallback, summary)
            } else {
                (hit.score.clone(), hit.reason.clone())
            };
            let candidate_count = hit.candidates.len();
            let score = if cached_score.total == 0.0 {
                hit.primary.confidence
            } else {
                cached_score.total
            };
            let strategy = hit.primary.strategy.clone();
            let elapsed = timer_start.elapsed();
            events::emit_resolve(&route, &strategy, score, candidate_count, true, elapsed);
            self.record_state_event(PerceiverEvent::resolve(
                route.clone(),
                strategy,
                score,
                candidate_count,
                true,
                to_score_records(&cached_score),
                cached_reason,
            ))
            .await;
            return Ok(hit);
        }

        let scope = Scope::Frame(route.frame.clone());
        let sampled_pair = match self.sample_for_resolve(&route, &scope, &selector).await {
            Some(pair) => Some(pair),
            None => {
                warn!(
                    "perceiver sampling failed after retries; continuing with hint-only candidates"
                );
                None
            }
        };

        let mut candidates = if let Some(ref legacy) = legacy_hint {
            match self.sampler.query(&route, legacy, &scope).await {
                Ok(list) => list,
                Err(err) => {
                    warn!(
                        ?err,
                        "perceiver query via CDP adapter failed; falling back to hint-only generation"
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        let fallback = generate::from_selector(&selector, &route.frame, &options);
        if candidates.is_empty() {
            debug!(
                ?selector,
                "perceiver query produced 0 candidates; using selector fallback"
            );
            candidates = fallback;
        } else if !fallback.is_empty() {
            candidates.extend(fallback);
        }
        if let Some(pair) = &sampled_pair {
            augment_candidates(&pair.dom, &pair.ax, &mut candidates);
        }
        if candidates.is_empty() {
            warn!(
                ?selector,
                "perceiver failed to produce candidates after fallback"
            );
            return Err(PerceiverError::AnchorNotFound("no candidates".into()));
        }
        let ranked = rank::rank_candidates(candidates, &policy.weights);
        let (top, top_score) = ranked
            .first()
            .cloned()
            .ok_or_else(|| PerceiverError::AnchorNotFound("no ranked candidate".into()))?;
        let limit = options.max_candidates.max(1);
        let shortlisted: Vec<AnchorDescriptor> = ranked
            .iter()
            .take(limit)
            .map(|(anchor, _)| anchor.clone())
            .collect();
        let reason_text = reason::summarize(&top_score);
        let mut resolution = AnchorResolution {
            primary: top,
            candidates: shortlisted,
            reason: reason_text,
            score: top_score.clone(),
        };
        self.ensure_anchor_metadata(&route, &mut resolution.primary)
            .await?;
        let candidate_count = resolution.candidates.len();
        let score_value = resolution.score.total;
        let strategy = resolution.primary.strategy.clone();
        let elapsed = timer_start.elapsed();
        events::emit_resolve(
            &route,
            &strategy,
            score_value,
            candidate_count,
            false,
            elapsed,
        );
        self.anchor_cache.put(cache_key, resolution.clone());
        self.record_state_event(PerceiverEvent::resolve(
            route.clone(),
            strategy,
            score_value,
            candidate_count,
            false,
            to_score_records(&resolution.score),
            resolution.reason.clone(),
        ))
        .await;
        Ok(resolution)
    }

    async fn is_visible(
        &self,
        route: ExecRoute,
        anchor: &mut AnchorDescriptor,
    ) -> Result<crate::model::JudgeReport, PerceiverError> {
        let timer_start = Instant::now();
        self.ensure_anchor_metadata(&route, anchor).await?;
        let policy = self.policy_view();
        let report = judges::visible(anchor, &policy.judge);
        let elapsed = timer_start.elapsed();
        events::emit_judge(&route, "visible", report.ok, &report.reason, elapsed);
        self.record_state_event(PerceiverEvent::judge(
            route.clone(),
            "visible".into(),
            report.ok,
            report.reason.clone(),
            redact::redact_value(report.facts.clone()),
        ))
        .await;
        Ok(report)
    }

    async fn is_clickable(
        &self,
        route: ExecRoute,
        anchor: &mut AnchorDescriptor,
    ) -> Result<crate::model::JudgeReport, PerceiverError> {
        let timer_start = Instant::now();
        self.ensure_anchor_metadata(&route, anchor).await?;
        let policy = self.policy_view();
        let report = judges::clickable(anchor, &policy.judge);
        let elapsed = timer_start.elapsed();
        events::emit_judge(&route, "clickable", report.ok, &report.reason, elapsed);
        self.record_state_event(PerceiverEvent::judge(
            route.clone(),
            "clickable".into(),
            report.ok,
            report.reason.clone(),
            redact::redact_value(report.facts.clone()),
        ))
        .await;
        Ok(report)
    }

    async fn is_enabled(
        &self,
        route: ExecRoute,
        anchor: &mut AnchorDescriptor,
    ) -> Result<crate::model::JudgeReport, PerceiverError> {
        let timer_start = Instant::now();
        self.ensure_anchor_metadata(&route, anchor).await?;
        let policy = self.policy_view();
        let report = judges::enabled(anchor, &policy.judge);
        let elapsed = timer_start.elapsed();
        events::emit_judge(&route, "enabled", report.ok, &report.reason, elapsed);
        self.record_state_event(PerceiverEvent::judge(
            route.clone(),
            "enabled".into(),
            report.ok,
            report.reason.clone(),
            redact::redact_value(report.facts.clone()),
        ))
        .await;
        Ok(report)
    }

    async fn snapshot_dom_ax(&self, route: ExecRoute) -> Result<DomAxSnapshot, PerceiverError> {
        let scope = Scope::Page(route.page.clone());
        self.snapshot_dom_ax_ext(route, scope, SnapLevel::Full)
            .await
    }

    async fn snapshot_dom_ax_ext(
        &self,
        route: ExecRoute,
        scope: Scope,
        level: SnapLevel,
    ) -> Result<DomAxSnapshot, PerceiverError> {
        let timer_start = Instant::now();
        let cache_key = snapshot_cache_key(&route, &scope, level);
        if let Some(hit) = self.snapshot_cache.get(&cache_key) {
            let elapsed = timer_start.elapsed();
            events::emit_snapshot(&route, true, elapsed);
            self.record_state_event(PerceiverEvent::snapshot(route.clone(), true))
                .await;
            return Ok(hit);
        }

        let snapshot = match self.sampler.sample(&route, &scope, level).await {
            Ok(pair) => DomAxSnapshot::new(
                route.page.clone(),
                route.frame.clone(),
                Some(route.session.clone()),
                level,
                pair.dom,
                pair.ax,
            ),
            Err(err) => {
                return Err(err);
            }
        };
        self.snapshot_cache.put(cache_key, snapshot.clone());
        let elapsed = timer_start.elapsed();
        events::emit_snapshot(&route, false, elapsed);
        self.record_state_event(PerceiverEvent::snapshot(route.clone(), false))
            .await;
        Ok(snapshot)
    }

    async fn diff_dom_ax(
        &self,
        route: ExecRoute,
        base: &DomAxSnapshot,
        current: &DomAxSnapshot,
    ) -> Result<DomAxDiff, PerceiverError> {
        let timer_start = Instant::now();
        let policy = self.policy_view();
        let diff = differ::diff_with_policy(base, current, Some(&policy.diff));
        if !diff.changes.is_empty() {
            self.invalidate_for_route(&route);
        }
        let elapsed = timer_start.elapsed();
        events::emit_diff(diff.changes.len(), elapsed);
        self.record_state_event(PerceiverEvent::diff(
            route.clone(),
            diff.changes.len(),
            redact_changes(&diff.changes),
        ))
        .await;
        Ok(diff)
    }
}

#[async_trait]
#[async_trait::async_trait]
impl<P> StructuralPerceiver for StructuralPerceiverImpl<P>
where
    P: CdpPerceptionPort + Send + Sync,
{
    async fn resolve_anchor(
        &self,
        route: ExecRoute,
        hint: ResolveHint,
        options: ResolveOptions,
    ) -> Result<AnchorResolution, PerceiverError> {
        let selector = SelectorOrHint::from(&hint);
        let policy = self.policy_view();
        let merged = merge_resolve_options(options, &policy.resolve);
        let opt_ext = ResolveOpt::from(&merged);
        self.resolve_with_selector(route, selector, opt_ext, Some(hint), &policy)
            .await
    }

    async fn resolve_anchor_ext(
        &self,
        route: ExecRoute,
        hint: SelectorOrHint,
        options: ResolveOpt,
    ) -> Result<AnchorResolution, PerceiverError> {
        let policy = self.policy_view();
        let merged = merge_resolve_opt(options, &policy.resolve);
        self.resolve_with_selector(route, hint, merged, None, &policy)
            .await
    }

    async fn is_visible(
        &self,
        route: ExecRoute,
        anchor: &mut AnchorDescriptor,
    ) -> Result<crate::model::JudgeReport, PerceiverError> {
        <StructuralPerceiverImpl<P>>::is_visible(self, route, anchor).await
    }

    async fn is_clickable(
        &self,
        route: ExecRoute,
        anchor: &mut AnchorDescriptor,
    ) -> Result<crate::model::JudgeReport, PerceiverError> {
        <StructuralPerceiverImpl<P>>::is_clickable(self, route, anchor).await
    }

    async fn is_enabled(
        &self,
        route: ExecRoute,
        anchor: &mut AnchorDescriptor,
    ) -> Result<crate::model::JudgeReport, PerceiverError> {
        <StructuralPerceiverImpl<P>>::is_enabled(self, route, anchor).await
    }

    async fn snapshot_dom_ax(&self, route: ExecRoute) -> Result<DomAxSnapshot, PerceiverError> {
        <StructuralPerceiverImpl<P>>::snapshot_dom_ax(self, route).await
    }

    async fn snapshot_dom_ax_ext(
        &self,
        route: ExecRoute,
        scope: Scope,
        level: SnapLevel,
    ) -> Result<DomAxSnapshot, PerceiverError> {
        <StructuralPerceiverImpl<P>>::snapshot_dom_ax_ext(self, route, scope, level).await
    }

    async fn diff_dom_ax(
        &self,
        route: ExecRoute,
        base: &DomAxSnapshot,
        current: &DomAxSnapshot,
    ) -> Result<DomAxDiff, PerceiverError> {
        <StructuralPerceiverImpl<P>>::diff_dom_ax(self, route, base, current).await
    }

    async fn diff_dom_ax_ext(
        &self,
        route: ExecRoute,
        base: &DomAxSnapshot,
        current: &DomAxSnapshot,
        focus: Option<crate::model::DiffFocus>,
    ) -> Result<DomAxDiff, PerceiverError> {
        let _ = focus;
        <StructuralPerceiverImpl<P>>::diff_dom_ax(self, route, base, current).await
    }
}

fn merge_resolve_options(mut options: ResolveOptions, defaults: &ResolveOptions) -> ResolveOptions {
    if options.max_candidates == 0 {
        options.max_candidates = defaults.max_candidates;
    }
    if options.max_candidates == 0 {
        options.max_candidates = 1;
    }
    if options.fuzziness.is_none() {
        options.fuzziness = defaults.fuzziness;
    }
    if options.debounce_ms.is_none() {
        options.debounce_ms = defaults.debounce_ms;
    }
    options
}

fn merge_resolve_opt(mut options: ResolveOpt, defaults: &ResolveOptions) -> ResolveOpt {
    if options.max_candidates == 0 {
        options.max_candidates = defaults.max_candidates;
    }
    if options.max_candidates == 0 {
        options.max_candidates = 1;
    }
    if options.fuzziness.is_none() {
        options.fuzziness = defaults.fuzziness;
    }
    if options.debounce_ms.is_none() {
        options.debounce_ms = defaults.debounce_ms;
    }
    options
}

fn snapshot_cache_key(route: &ExecRoute, scope: &Scope, level: SnapLevel) -> String {
    let scope_key = match scope {
        Scope::Frame(frame) => format!("frame:{}", frame.0),
        Scope::Page(page) => format!("page:{}", page.0),
    };
    let level_key = match level {
        SnapLevel::Light => "light",
        SnapLevel::Full => "full",
    };
    format!("snapshot:{}:{}:{}", route.page.0, scope_key, level_key)
}

fn augment_candidates(
    dom_snapshot: &Value,
    ax_snapshot: &Value,
    candidates: &mut [AnchorDescriptor],
) {
    if candidates.is_empty() || dom_snapshot.is_null() {
        return;
    }

    let dom_index = DomIndex::from_snapshot(dom_snapshot);
    let ax_index = AxIndex::from_snapshot(ax_snapshot);
    for anchor in candidates.iter_mut() {
        if let Some(backend) = anchor.backend_node_id {
            if let Some(meta) = dom_index.metadata.get(&backend) {
                anchor.value = merge_json_objects(anchor.value.clone(), meta.clone());
            }
            if let Some(ax_meta) = ax_index.metadata.get(&backend) {
                anchor.value = merge_json_objects(anchor.value.clone(), ax_meta.clone());
            }
            if let Some(geom) = dom_index.geometry.get(&backend) {
                anchor.geometry = Some(geom.clone());
            }
            continue;
        }

        if let Some(current_geom) = &anchor.geometry {
            if let Some((backend, geom, meta)) = dom_index.nearest_to(current_geom) {
                anchor.backend_node_id = Some(backend);
                anchor.geometry = Some(geom);
                anchor.value = merge_json_objects(anchor.value.clone(), meta);
                if let Some(ax_meta) = ax_index.metadata.get(&backend) {
                    anchor.value = merge_json_objects(anchor.value.clone(), ax_meta.clone());
                }
                continue;
            }
        }

        // fallback: pick first node from metadata to keep anchor usable
        if anchor.backend_node_id.is_none() {
            if let Some((&backend, meta)) = dom_index.metadata.iter().next() {
                anchor.backend_node_id = Some(backend);
                anchor.geometry = dom_index.geometry.get(&backend).cloned();
                anchor.value = merge_json_objects(anchor.value.clone(), meta.clone());
                if let Some(ax_meta) = ax_index.metadata.get(&backend) {
                    anchor.value = merge_json_objects(anchor.value.clone(), ax_meta.clone());
                }
            }
        }
    }
}

fn to_score_records(score: &ScoreBreakdown) -> Vec<ScoreComponentRecord> {
    score
        .components
        .iter()
        .map(|component| ScoreComponentRecord {
            label: component.label.clone(),
            weight: component.weight,
            contribution: component.contribution,
        })
        .collect()
}

fn redact_changes(changes: &[Value]) -> Vec<Value> {
    match redact::redact_value(Value::Array(changes.to_vec())) {
        Value::Array(items) => items,
        other => vec![other],
    }
}

fn merge_json_objects(primary: Value, secondary: Value) -> Value {
    match (primary, secondary) {
        (Value::Object(mut base), Value::Object(extra)) => {
            for (key, value) in extra {
                base.entry(key).or_insert(value);
            }
            Value::Object(base)
        }
        (Value::Object(base), other) => {
            let mut map = base;
            map.entry("extra".to_string()).or_insert(other);
            Value::Object(map)
        }
        (other, Value::Object(extra)) => {
            let mut map = JsonMap::new();
            map.insert("original".to_string(), other);
            for (key, value) in extra {
                map.entry(key).or_insert(value);
            }
            Value::Object(map)
        }
        (left, right) => Value::Array(vec![left, right]),
    }
}

fn extract_ax_string(entry: Option<&Value>) -> Option<String> {
    let value = entry?;
    if let Some(obj) = value.as_object() {
        if let Some(inner) = obj.get("value") {
            return extract_ax_string(Some(inner));
        }
        if let Some(inner) = obj.get("stringValue") {
            return inner.as_str().map(|s| s.to_string());
        }
    }
    value.as_str().map(|s| s.to_string())
}

fn extract_state_flags(entries: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    for entry in entries {
        let obj = match entry.as_object() {
            Some(obj) => obj,
            None => continue,
        };
        let name = match obj.get("name").and_then(Value::as_str) {
            Some(name) => name,
            None => continue,
        };
        let is_true = obj
            .get("value")
            .and_then(extract_bool)
            .or_else(|| obj.get("boolValue").and_then(Value::as_bool))
            .unwrap_or(false);
        if is_true {
            out.push(name.to_string());
        }
    }
    out
}

fn extract_bool(value: &Value) -> Option<bool> {
    if let Some(flag) = value.as_bool() {
        return Some(flag);
    }
    if let Some(obj) = value.as_object() {
        if let Some(inner) = obj.get("value") {
            return extract_bool(inner);
        }
        if let Some(flag) = obj.get("boolValue").and_then(Value::as_bool) {
            return Some(flag);
        }
    }
    None
}

struct DomIndex {
    metadata: HashMap<u64, Value>,
    geometry: HashMap<u64, AnchorGeometry>,
}

impl DomIndex {
    fn from_snapshot(snapshot: &Value) -> Self {
        let mut metadata = HashMap::new();
        let mut geometry = HashMap::new();

        let strings = snapshot
            .get("strings")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if let Some(documents) = snapshot.get("documents").and_then(|v| v.as_array()) {
            for document in documents {
                Self::extract_document(document, &strings, &mut metadata, &mut geometry);
            }
        }

        Self { metadata, geometry }
    }

    fn extract_document(
        document: &Value,
        strings: &[Value],
        metadata: &mut HashMap<u64, Value>,
        geometry: &mut HashMap<u64, AnchorGeometry>,
    ) {
        let nodes_obj = match document.get("nodes").and_then(|v| v.as_object()) {
            Some(obj) => obj,
            None => return,
        };

        let backend_ids = match nodes_obj.get("backendNodeId").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => return,
        };

        let backend_list: Vec<u64> = backend_ids.iter().filter_map(|v| v.as_u64()).collect();
        if backend_list.is_empty() {
            return;
        }

        let node_names = nodes_obj.get("nodeName").and_then(|v| v.as_array());
        let node_values = nodes_obj.get("nodeValue").and_then(|v| v.as_array());
        let attributes = nodes_obj.get("attributes").and_then(|v| v.as_array());

        for (idx, backend_id) in backend_list.iter().enumerate() {
            let mut meta = JsonMap::new();
            if let Some(name_arr) = node_names {
                if let Some(name_val) = name_arr.get(idx) {
                    if let Some(name) = decode_indexed_string(strings, name_val) {
                        meta.insert("nodeName".to_string(), Value::String(name));
                    }
                }
            }
            if let Some(value_arr) = node_values {
                if let Some(node_val) = value_arr.get(idx) {
                    if let Some(text) = decode_indexed_string(strings, node_val) {
                        if !text.trim().is_empty() {
                            meta.insert("nodeValue".to_string(), Value::String(text));
                        }
                    }
                }
            }
            if let Some(attrs_arr) = attributes {
                if let Some(entry) = attrs_arr.get(idx).and_then(|v| v.as_array()) {
                    let mut attr_map = JsonMap::new();
                    let mut iter = entry.iter();
                    while let Some(name_idx) = iter.next() {
                        if let Some(value_idx) = iter.next() {
                            if let Some(name) = decode_indexed_string(strings, name_idx) {
                                let value =
                                    decode_indexed_string(strings, value_idx).unwrap_or_default();
                                attr_map.insert(name, Value::String(value));
                            }
                        }
                    }
                    if !attr_map.is_empty() {
                        meta.insert("attributes".to_string(), Value::Object(attr_map));
                    }
                }
            }

            if !meta.is_empty() {
                metadata.insert(*backend_id, Value::Object(meta));
            }
        }

        if let Some(layout) = document.get("layout").and_then(|v| v.as_object()) {
            let node_index = layout.get("nodeIndex").and_then(|v| v.as_array());
            let bounds = layout.get("bounds").and_then(|v| v.as_array());
            if let (Some(node_index), Some(bounds)) = (node_index, bounds) {
                for (i, node_idx_val) in node_index.iter().enumerate() {
                    let node_idx = match node_idx_val.as_u64().and_then(|v| usize::try_from(v).ok())
                    {
                        Some(idx) => idx,
                        None => continue,
                    };
                    let backend_id = match backend_list.get(node_idx) {
                        Some(id) => id,
                        None => continue,
                    };
                    let base = i * 4;
                    if bounds.len() < base + 4 {
                        continue;
                    }
                    let x = bounds[base].as_f64().unwrap_or(0.0);
                    let y = bounds[base + 1].as_f64().unwrap_or(0.0);
                    let width = bounds[base + 2].as_f64().unwrap_or(0.0);
                    let height = bounds[base + 3].as_f64().unwrap_or(0.0);
                    geometry.insert(
                        *backend_id,
                        AnchorGeometry {
                            x,
                            y,
                            width,
                            height,
                        },
                    );
                }
            }
        }
    }

    fn nearest_to(&self, target: &AnchorGeometry) -> Option<(u64, AnchorGeometry, Value)> {
        let target_cx = target.x + target.width / 2.0;
        let target_cy = target.y + target.height / 2.0;
        let mut best: Option<(u64, f64)> = None;
        for (backend, geom) in &self.geometry {
            let cx = geom.x + geom.width / 2.0;
            let cy = geom.y + geom.height / 2.0;
            let dist = (cx - target_cx).powi(2) + (cy - target_cy).powi(2);
            if dist.is_nan() {
                continue;
            }
            match best {
                Some((_, best_dist)) if dist >= best_dist => {}
                _ => best = Some((*backend, dist)),
            }
        }

        if let Some((backend, _)) = best {
            let geom = self.geometry.get(&backend)?.clone();
            let meta = self.metadata.get(&backend).cloned().unwrap_or(Value::Null);
            Some((backend, geom, meta))
        } else {
            None
        }
    }
}

struct AxIndex {
    metadata: HashMap<u64, Value>,
}

impl AxIndex {
    fn from_snapshot(snapshot: &Value) -> Self {
        if snapshot.is_null() {
            return Self {
                metadata: HashMap::new(),
            };
        }

        let mut metadata = HashMap::new();
        if let Some(nodes) = snapshot.get("nodes").and_then(Value::as_array) {
            for node in nodes {
                let backend_id = match node
                    .get("backendDOMNodeId")
                    .and_then(Value::as_u64)
                    .or_else(|| node.get("backendNodeId").and_then(Value::as_u64))
                {
                    Some(id) => id,
                    None => continue,
                };

                let mut meta = JsonMap::new();
                if let Some(role) = extract_ax_string(node.get("role")) {
                    meta.insert("axRole".into(), Value::String(role));
                }
                if let Some(name) = extract_ax_string(node.get("name")) {
                    meta.insert("axName".into(), Value::String(name));
                }
                if let Some(value) = extract_ax_string(node.get("value")) {
                    meta.insert("axValue".into(), Value::String(value));
                }

                let mut states = Vec::new();
                if let Some(state_entries) = node.get("states").and_then(Value::as_array) {
                    states.extend(extract_state_flags(state_entries));
                }
                if let Some(props) = node.get("properties").and_then(Value::as_array) {
                    states.extend(extract_state_flags(props));
                }
                if !states.is_empty() {
                    states.sort();
                    states.dedup();
                    meta.insert(
                        "axStates".into(),
                        Value::Array(states.into_iter().map(Value::String).collect()),
                    );
                }

                if !meta.is_empty() {
                    metadata.insert(backend_id, Value::Object(meta));
                }
            }
        }

        Self { metadata }
    }
}

fn decode_indexed_string(strings: &[Value], value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(num) => num.as_u64().and_then(|idx| {
            strings
                .get(idx as usize)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::model::SampledPair;
    use async_trait::async_trait;
    use serde_json::json;
    use soulbrowser_core_types::{FrameId, PageId, SessionId};
    use soulbrowser_state_center::{InMemoryStateCenter, StateCenter, StateEvent};

    struct StubPort;

    #[async_trait]
    impl CdpPerceptionPort for StubPort {
        async fn sample_dom_ax(
            &self,
            _route: &ExecRoute,
            _scope: &Scope,
            _level: SnapLevel,
        ) -> Result<SampledPair, PerceiverError> {
            Ok(SampledPair {
                dom: json!({
                    "strings": [],
                    "documents": [
                        {
                            "nodes": {
                                "nodeName": ["BUTTON"],
                                "nodeValue": ["Submit"],
                                "attributes": [["class", "btn"]],
                            },
                            "layout": {
                                "nodeIndex": [0],
                                "bounds": [0.0, 0.0, 120.0, 32.0],
                            }
                        }
                    ]
                }),
                ax: json!({
                    "nodes": [
                        {
                            "backendDOMNodeId": 7,
                            "role": { "value": "button" },
                            "actions": [{ "name": "focus" }]
                        }
                    ]
                }),
            })
        }

        async fn query(
            &self,
            route: &ExecRoute,
            hint: &ResolveHint,
            _scope: &Scope,
        ) -> Result<Vec<AnchorDescriptor>, PerceiverError> {
            match hint {
                ResolveHint::Css(selector) => Ok(vec![AnchorDescriptor {
                    strategy: "css".into(),
                    value: json!({
                        "selector": selector,
                        "nodeName": "BUTTON",
                        "attributes": { "class": "btn" },
                    }),
                    frame_id: route.frame.clone(),
                    confidence: 0.7,
                    backend_node_id: Some(7),
                    geometry: Some(AnchorGeometry {
                        x: 0.0,
                        y: 0.0,
                        width: 120.0,
                        height: 32.0,
                    }),
                }]),
                _ => Ok(Vec::new()),
            }
        }

        async fn describe_backend_node(
            &self,
            _route: &ExecRoute,
            backend_node_id: u64,
        ) -> Result<Value, PerceiverError> {
            Ok(json!({
                "backendNodeId": backend_node_id,
                "attributes": { "class": "btn" },
                "style": { "display": "block" },
                "geometry": { "x": 0.0, "y": 0.0, "width": 120.0, "height": 32.0 },
            }))
        }

        async fn node_attributes(
            &self,
            _route: &ExecRoute,
            _backend_node_id: u64,
        ) -> Result<Option<Value>, PerceiverError> {
            Ok(Some(json!({
                "class": "btn"
            })))
        }

        async fn node_style(
            &self,
            _route: &ExecRoute,
            _backend_node_id: u64,
        ) -> Result<Option<Value>, PerceiverError> {
            Ok(Some(json!({
                "display": "block"
            })))
        }
    }

    fn make_route() -> ExecRoute {
        ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new())
    }

    #[tokio::test]
    async fn state_center_records_perceiver_events() {
        let port = Arc::new(StubPort);
        let state_center = Arc::new(InMemoryStateCenter::new(32));
        let state_center_dyn: Arc<dyn StateCenter> = state_center.clone();
        let perceiver =
            StructuralPerceiverImpl::with_state_center(Arc::clone(&port), state_center_dyn);

        let route = make_route();
        let resolve_opts = ResolveOptions {
            max_candidates: 3,
            ..Default::default()
        };
        let mut resolution = perceiver
            .resolve_anchor(
                route.clone(),
                ResolveHint::Css("button".into()),
                resolve_opts,
            )
            .await
            .expect("resolve anchor");

        perceiver
            .is_visible(route.clone(), &mut resolution.primary)
            .await
            .expect("visible");
        perceiver
            .is_clickable(route.clone(), &mut resolution.primary)
            .await
            .expect("clickable");
        perceiver
            .is_enabled(route.clone(), &mut resolution.primary)
            .await
            .expect("enabled");

        let snapshot = perceiver
            .snapshot_dom_ax(route.clone())
            .await
            .expect("snapshot");
        perceiver
            .snapshot_dom_ax(route.clone())
            .await
            .expect("snapshot cache hit");

        perceiver
            .diff_dom_ax(route.clone(), &snapshot, &snapshot)
            .await
            .expect("diff");

        let stats = state_center.stats();
        assert_eq!(stats.perceiver_resolve, 1);
        assert_eq!(stats.perceiver_judge, 3);
        assert_eq!(stats.perceiver_snapshot, 2);
        assert_eq!(stats.perceiver_diff, 1);

        let session_events = state_center.recent_session(&route.session);
        assert!(session_events
            .iter()
            .any(|event| matches!(event, StateEvent::Perceiver(_))));
        let page_events = state_center.recent_page(&route.page);
        assert_eq!(page_events.len(), session_events.len());
    }
}
