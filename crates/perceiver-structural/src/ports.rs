use std::sync::Arc;

use async_trait::async_trait;
use cdp_adapter::commands::{QueryScope, QuerySpec};
use cdp_adapter::ids::PageId as AdapterPageId;
use cdp_adapter::{AxSnapshotConfig, Cdp, DomSnapshotConfig};
use serde_json::json;
use soulbrowser_core_types::ExecRoute;
use uuid::Uuid;

use crate::errors::PerceiverError;
use crate::model::{AnchorDescriptor, AnchorGeometry, ResolveHint, SampledPair};

#[async_trait]
pub trait CdpPerceptionPort: Send + Sync {
    async fn sample_dom_ax(&self, route: &ExecRoute) -> Result<SampledPair, PerceiverError>;
    async fn query(
        &self,
        route: &ExecRoute,
        hint: &ResolveHint,
    ) -> Result<Vec<AnchorDescriptor>, PerceiverError>;
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
}

#[async_trait]
impl<C> CdpPerceptionPort for AdapterPort<C>
where
    C: Cdp + Send + Sync,
{
    async fn sample_dom_ax(&self, route: &ExecRoute) -> Result<SampledPair, PerceiverError> {
        let page = Self::parse_page(route)?;
        let dom = self
            .adapter
            .dom_snapshot(page, DomSnapshotConfig::default())
            .await
            .map_err(|err| {
                PerceiverError::internal(format!("dom snapshot failed: {:?}", err.kind))
            })?;
        let ax = self
            .adapter
            .ax_snapshot(page, AxSnapshotConfig::default())
            .await
            .map_err(|err| {
                PerceiverError::internal(format!("ax snapshot failed: {:?}", err.kind))
            })?;

        Ok(SampledPair {
            dom: dom.raw,
            ax: ax.raw,
        })
    }

    async fn query(
        &self,
        route: &ExecRoute,
        hint: &ResolveHint,
    ) -> Result<Vec<AnchorDescriptor>, PerceiverError> {
        let page = Self::parse_page(route)?;
        let frame_id = route.frame.clone();

        let (strategy, selector) = match hint {
            ResolveHint::Css(sel) => ("css".to_string(), sel.clone()),
            _ => return Ok(Vec::new()),
        };

        let selector_value = selector.clone();
        let result = self
            .adapter
            .query(
                page,
                QuerySpec {
                    selector: selector.clone(),
                    scope: QueryScope::Document,
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
}
