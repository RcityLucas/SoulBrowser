use std::sync::Arc;

use async_trait::async_trait;
use cdp_adapter::commands::{AxSnapshotConfig, DomSnapshotConfig, QueryScope, QuerySpec};
use cdp_adapter::ids::PageId as AdapterPageId;
use cdp_adapter::Cdp;
use serde_json::{json, Map as JsonMap, Value};
use soulbrowser_core_types::ExecRoute;
use uuid::Uuid;

use crate::errors::PerceiverError;
use crate::model::{AnchorDescriptor, AnchorGeometry, ResolveHint, SampledPair, Scope, SnapLevel};

#[async_trait]
pub trait CdpPerceptionPort: Send + Sync {
    async fn sample_dom_ax(
        &self,
        route: &ExecRoute,
        scope: &Scope,
        level: SnapLevel,
    ) -> Result<SampledPair, PerceiverError>;
    async fn query(
        &self,
        route: &ExecRoute,
        hint: &ResolveHint,
        scope: &Scope,
    ) -> Result<Vec<AnchorDescriptor>, PerceiverError>;
    async fn describe_backend_node(
        &self,
        route: &ExecRoute,
        backend_node_id: u64,
    ) -> Result<Value, PerceiverError>;
    async fn node_attributes(
        &self,
        route: &ExecRoute,
        backend_node_id: u64,
    ) -> Result<Option<Value>, PerceiverError>;
    async fn node_style(
        &self,
        route: &ExecRoute,
        backend_node_id: u64,
    ) -> Result<Option<Value>, PerceiverError>;
}

pub struct AdapterPort<C>
where
    C: Cdp + Send + Sync,
{
    adapter: Arc<C>,
}

impl<C> AdapterPort<C>
where
    C: Cdp + Send + Sync,
{
    pub fn new(adapter: Arc<C>) -> Self {
        Self { adapter }
    }

    fn parse_page(route: &ExecRoute) -> Result<AdapterPageId, PerceiverError> {
        let id = Uuid::parse_str(&route.page.0)
            .map_err(|err| PerceiverError::internal(format!("invalid page id: {err}")))?;
        Ok(AdapterPageId(id))
    }

    async fn backend_description(
        &self,
        route: &ExecRoute,
        backend_node_id: u64,
    ) -> Result<JsonMap<String, Value>, PerceiverError> {
        let page = Self::parse_page(route)?;
        let dom = self
            .adapter
            .dom_snapshot(page, DomSnapshotConfig::default())
            .await
            .map_err(|err| {
                PerceiverError::internal(format!("dom snapshot failed: {:?}", err.kind))
            })?;
        let strings: Vec<Value> = dom.strings.into_iter().map(|s| Value::String(s)).collect();
        describe_backend(&dom.raw, &strings, backend_node_id).ok_or_else(|| {
            PerceiverError::AnchorNotFound("backend node not present in snapshot".into())
        })
    }
}

#[async_trait]
impl<C> CdpPerceptionPort for AdapterPort<C>
where
    C: Cdp + Send + Sync,
{
    async fn sample_dom_ax(
        &self,
        route: &ExecRoute,
        scope: &Scope,
        level: SnapLevel,
    ) -> Result<SampledPair, PerceiverError> {
        let page = Self::parse_page(route)?;
        let dom_config = match level {
            SnapLevel::Light => {
                let mut cfg = DomSnapshotConfig::default();
                cfg.computed_style_whitelist = vec![
                    "display".into(),
                    "visibility".into(),
                    "opacity".into(),
                    "pointer-events".into(),
                ];
                cfg
            }
            SnapLevel::Full => DomSnapshotConfig::default(),
        };
        let dom = self
            .adapter
            .dom_snapshot(page, dom_config)
            .await
            .map_err(|err| {
                PerceiverError::internal(format!("dom snapshot failed: {:?}", err.kind))
            })?;

        let ax_raw = if matches!(level, SnapLevel::Light) {
            Value::Null
        } else {
            let mut ax_config = AxSnapshotConfig::default();
            if let Scope::Frame(frame_id) = scope {
                ax_config.frame_id = Some(frame_id.0.clone());
            }
            self.adapter
                .ax_snapshot(page, ax_config)
                .await
                .map_err(|err| {
                    PerceiverError::internal(format!("ax snapshot failed: {:?}", err.kind))
                })?
                .raw
        };

        Ok(SampledPair {
            dom: dom.raw,
            ax: ax_raw,
        })
    }

    async fn query(
        &self,
        route: &ExecRoute,
        hint: &ResolveHint,
        scope: &Scope,
    ) -> Result<Vec<AnchorDescriptor>, PerceiverError> {
        let page = Self::parse_page(route)?;
        let frame_id = route.frame.clone();

        let (strategy, selector) = match hint {
            ResolveHint::Css(sel) => ("css".to_string(), sel.clone()),
            _ => return Ok(Vec::new()),
        };

        let selector_value = selector.clone();
        let query_scope = match scope {
            Scope::Frame(frame) => QueryScope::Frame(frame.0.clone()),
            Scope::Page(_) => QueryScope::Document,
        };
        let result = self
            .adapter
            .query(
                page,
                QuerySpec {
                    selector: selector.clone(),
                    scope: query_scope,
                },
            )
            .await
            .map_err(|err| PerceiverError::internal(format!("query failed: {:?}", err.kind)))?;

        let anchors = result
            .into_iter()
            .map(|anchor| AnchorDescriptor {
                strategy: strategy.clone(),
                value: json!({
                    "selector": selector_value.clone(),
                    "backendNodeId": anchor.backend_node_id,
                    "x": anchor.x,
                    "y": anchor.y,
                }),
                frame_id: frame_id.clone(),
                confidence: 0.75,
                backend_node_id: anchor.backend_node_id,
                geometry: Some(AnchorGeometry {
                    x: anchor.x,
                    y: anchor.y,
                    width: 0.0,
                    height: 0.0,
                }),
            })
            .collect();

        Ok(anchors)
    }

    async fn describe_backend_node(
        &self,
        route: &ExecRoute,
        backend_node_id: u64,
    ) -> Result<Value, PerceiverError> {
        let description = self.backend_description(route, backend_node_id).await?;
        Ok(Value::Object(description))
    }

    async fn node_attributes(
        &self,
        route: &ExecRoute,
        backend_node_id: u64,
    ) -> Result<Option<Value>, PerceiverError> {
        let description = self.backend_description(route, backend_node_id).await?;
        Ok(description.get("attributes").cloned())
    }

    async fn node_style(
        &self,
        route: &ExecRoute,
        backend_node_id: u64,
    ) -> Result<Option<Value>, PerceiverError> {
        let description = self.backend_description(route, backend_node_id).await?;
        Ok(description.get("style").cloned())
    }
}

fn describe_backend(
    raw: &Value,
    strings: &[Value],
    backend_id_target: u64,
) -> Option<JsonMap<String, Value>> {
    let mut out = JsonMap::new();
    let documents = raw.get("documents")?.as_array()?;
    for document in documents {
        let nodes = document.get("nodes")?.as_object()?;
        let backend_ids = nodes.get("backendNodeId")?.as_array()?;
        let node_names = nodes.get("nodeName").and_then(Value::as_array);
        let node_values = nodes.get("nodeValue").and_then(Value::as_array);
        let attributes = nodes.get("attributes").and_then(Value::as_array);
        let node_style_refs = nodes.get("computedStyles").and_then(Value::as_array);
        let computed_styles = document.get("computedStyles").and_then(Value::as_array);

        for (idx, backend) in backend_ids.iter().enumerate() {
            if backend.as_u64() != Some(backend_id_target) {
                continue;
            }

            if let Some(node_names) = node_names {
                if let Some(name) = node_names
                    .get(idx)
                    .and_then(|value| decode_string(strings, value))
                {
                    out.insert("nodeName".into(), Value::String(name.to_ascii_uppercase()));
                }
            }
            if let Some(node_values) = node_values {
                if let Some(value) = node_values
                    .get(idx)
                    .and_then(|value| decode_string(strings, value))
                {
                    if !value.trim().is_empty() {
                        out.insert("nodeValue".into(), Value::String(value));
                    }
                }
            }
            if let Some(attributes) = attributes {
                if let Some(entry) = attributes.get(idx).and_then(Value::as_array) {
                    let mut attrs = JsonMap::new();
                    let mut iter = entry.iter();
                    while let Some(name_idx) = iter.next() {
                        if let Some(value_idx) = iter.next() {
                            if let Some(name) = decode_string(strings, name_idx) {
                                let value = decode_string(strings, value_idx).unwrap_or_default();
                                attrs.insert(name, Value::String(value));
                            }
                        }
                    }
                    if !attrs.is_empty() {
                        out.insert("attributes".into(), Value::Object(attrs));
                    }
                }
            }

            if let (Some(node_style_refs), Some(computed_styles)) =
                (node_style_refs.as_ref(), computed_styles)
            {
                if let Some(style_idx_value) = node_style_refs.get(idx) {
                    let style_indices: Vec<u64> = match style_idx_value {
                        Value::Number(num) => num.as_u64().into_iter().collect(),
                        Value::Array(values) => values.iter().filter_map(|v| v.as_u64()).collect(),
                        _ => Vec::new(),
                    };
                    for style_index in style_indices {
                        if let Some(entry) = computed_styles
                            .get(style_index as usize)
                            .and_then(Value::as_object)
                        {
                            let properties = entry.get("properties").and_then(Value::as_array);
                            let values = entry.get("values").and_then(Value::as_array);
                            if let (Some(properties), Some(values)) = (properties, values) {
                                let mut style_map = JsonMap::new();
                                for (prop_idx, value_idx) in properties.iter().zip(values.iter()) {
                                    if let Some(name) = decode_string(strings, prop_idx) {
                                        if let Some(value) = decode_string(strings, value_idx) {
                                            style_map.insert(name, Value::String(value));
                                        }
                                    }
                                }
                                if !style_map.is_empty() {
                                    let entry = out
                                        .entry("style".to_string())
                                        .or_insert_with(|| Value::Object(JsonMap::new()));
                                    if let Value::Object(existing) = entry {
                                        for (key, value) in style_map {
                                            existing.entry(key).or_insert(value);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if let Some(layout) = document.get("layout").and_then(Value::as_object) {
                if let (Some(node_index), Some(bounds)) = (
                    layout.get("nodeIndex").and_then(Value::as_array),
                    layout.get("bounds").and_then(Value::as_array),
                ) {
                    if let Some(position) = node_index
                        .iter()
                        .position(|value| value.as_u64() == Some(idx as u64))
                    {
                        let base = position * 4;
                        if bounds.len() >= base + 4 {
                            let x = bounds[base].as_f64().unwrap_or(0.0);
                            let y = bounds[base + 1].as_f64().unwrap_or(0.0);
                            let width = bounds[base + 2].as_f64().unwrap_or(0.0);
                            let height = bounds[base + 3].as_f64().unwrap_or(0.0);
                            out.insert(
                                "geometry".into(),
                                json!({
                                    "x": x,
                                    "y": y,
                                    "width": width,
                                    "height": height,
                                }),
                            );
                        }
                    }
                }
            }

            return Some(out);
        }
    }
    None
}

fn decode_string(strings: &[Value], value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(num) => num.as_u64().and_then(|idx| {
            strings
                .get(idx as usize)
                .and_then(|entry| entry.as_str())
                .map(|s| s.to_string())
        }),
        _ => None,
    }
}
