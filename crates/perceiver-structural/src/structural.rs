use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
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
};
use crate::policy::ResolveOptions;
use crate::ports::CdpPerceptionPort;
use crate::resolver::{generate, rank};
use crate::sampler::Sampler;
use tracing::{debug, warn};

pub struct StructuralPerceiverImpl<P>
where
    P: CdpPerceptionPort + Send + Sync,
{
    sampler: Sampler<P>,
    anchor_cache: AnchorCache,
    snapshot_cache: SnapshotCache,
}

impl<P> StructuralPerceiverImpl<P>
where
    P: CdpPerceptionPort + Send + Sync,
{
    pub fn new(port: Arc<P>) -> Self {
        Self {
            sampler: Sampler::new(port),
            anchor_cache: AnchorCache::new(std::time::Duration::from_millis(250)),
            snapshot_cache: SnapshotCache::new(std::time::Duration::from_secs(1)),
        }
    }
}

#[async_trait]
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
        let cache_key = format!("{:?}::{}", route.page, hint.cache_key());
        if let Some(hit) = self.anchor_cache.get(&cache_key) {
            return Ok(hit);
        }

        let sampled_pair = match self.sampler.sample(&route).await {
            Ok(pair) => Some(pair),
            Err(err) => {
                warn!(
                    ?err,
                    "perceiver dom/ax sampling failed; continuing with limited context"
                );
                None
            }
        };

        let mut candidates = match self.sampler.query(&route, &hint).await {
            Ok(list) => list,
            Err(err) => {
                warn!(
                    ?err,
                    "perceiver query via CDP adapter failed; falling back to hint-only generation"
                );
                Vec::new()
            }
        };
        if candidates.is_empty() {
            debug!(
                ?hint,
                "perceiver query produced 0 candidates; using hint fallback"
            );
            candidates = generate::from_hint(&hint);
        }
        if let Some(pair) = &sampled_pair {
            augment_candidates(&pair.dom, &mut candidates);
        }
        if candidates.is_empty() {
            warn!(
                ?hint,
                "perceiver failed to produce candidates after fallback"
            );
            return Err(PerceiverError::AnchorNotFound("no candidates".into()));
        }
        let top = rank::select_top(candidates.clone())
            .ok_or_else(|| PerceiverError::AnchorNotFound("no ranked candidate".into()))?;
        let limit = options.max_candidates.max(1);
        let resolution = AnchorResolution {
            primary: top,
            candidates: candidates.into_iter().take(limit).collect(),
            reason: "stub resolution".into(),
        };
        events::emit_resolve();
        self.anchor_cache.put(cache_key, resolution.clone());
        Ok(resolution)
    }

    async fn is_visible(
        &self,
        _route: ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<crate::model::JudgeReport, PerceiverError> {
        events::emit_judge();
        Ok(judges::visible(anchor))
    }

    async fn is_clickable(
        &self,
        _route: ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<crate::model::JudgeReport, PerceiverError> {
        events::emit_judge();
        Ok(judges::clickable(anchor))
    }

    async fn is_enabled(
        &self,
        _route: ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<crate::model::JudgeReport, PerceiverError> {
        events::emit_judge();
        Ok(judges::enabled(anchor))
    }

    async fn snapshot_dom_ax(&self, route: ExecRoute) -> Result<DomAxSnapshot, PerceiverError> {
        let cache_key = format!("snapshot:{:?}", route.page);
        if let Some(hit) = self.snapshot_cache.get(&cache_key) {
            return Ok(hit);
        }

        let snapshot = match self.sampler.sample(&route).await {
            Ok(pair) => DomAxSnapshot {
                dom_raw: pair.dom,
                ax_raw: pair.ax,
            },
            Err(err) => {
                warn!(
                    ?err,
                    "perceiver snapshot sample failed; returning empty snapshot"
                );
                DomAxSnapshot {
                    dom_raw: Value::Null,
                    ax_raw: Value::Null,
                }
            }
        };
        events::emit_snapshot();
        self.snapshot_cache.put(cache_key, snapshot.clone());
        Ok(snapshot)
    }

    async fn diff_dom_ax(
        &self,
        base: &DomAxSnapshot,
        current: &DomAxSnapshot,
    ) -> Result<DomAxDiff, PerceiverError> {
        events::emit_diff();
        Ok(differ::diff(base, current))
    }
}

fn augment_candidates(dom_snapshot: &Value, candidates: &mut [AnchorDescriptor]) {
    if candidates.is_empty() {
        return;
    }

    let index = DomIndex::from_snapshot(dom_snapshot);
    for anchor in candidates.iter_mut() {
        if let Some(backend) = anchor.backend_node_id {
            if let Some(meta) = index.metadata.get(&backend) {
                anchor.value = merge_json_objects(anchor.value.clone(), meta.clone());
            }
            if let Some(geom) = index.geometry.get(&backend) {
                anchor.geometry = Some(geom.clone());
            }
        }
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
