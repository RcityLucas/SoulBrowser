use async_trait::async_trait;
use soulbrowser_core_types::ExecRoute;

use crate::errors::PerceiverError;
use crate::model::{
    AnchorDescriptor, AnchorResolution, DomAxDiff, DomAxSnapshot, JudgeReport, ResolveHint,
};
use crate::policy::ResolveOptions;

#[async_trait]
pub trait StructuralPerceiver: Send + Sync {
    async fn resolve_anchor(
        &self,
        route: ExecRoute,
        hint: ResolveHint,
        options: ResolveOptions,
    ) -> Result<AnchorResolution, PerceiverError>;

    async fn is_visible(
        &self,
        route: ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<JudgeReport, PerceiverError>;

    async fn is_clickable(
        &self,
        route: ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<JudgeReport, PerceiverError>;

    async fn is_enabled(
        &self,
        route: ExecRoute,
        anchor: &AnchorDescriptor,
    ) -> Result<JudgeReport, PerceiverError>;

    async fn snapshot_dom_ax(&self, route: ExecRoute) -> Result<DomAxSnapshot, PerceiverError>;

    async fn diff_dom_ax(
        &self,
        base: &DomAxSnapshot,
        current: &DomAxSnapshot,
    ) -> Result<DomAxDiff, PerceiverError>;
}
