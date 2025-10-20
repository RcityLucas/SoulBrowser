use async_trait::async_trait;
use soulbrowser_core_types::ExecRoute;

use crate::errors::PerceiverError;
use crate::model::{
    AnchorDescriptor, AnchorResolution, DiffFocus, DomAxDiff, DomAxSnapshot, InteractionAdvice,
    JudgeReport, ResolveHint, ResolveOpt, Scope, SelectorOrHint, SnapLevel,
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

    async fn resolve_anchor_ext(
        &self,
        route: ExecRoute,
        hint: SelectorOrHint,
        options: ResolveOpt,
    ) -> Result<AnchorResolution, PerceiverError> {
        let legacy_hint = ResolveHint::from(&hint);
        let legacy_opts = ResolveOptions::from(&options);
        self.resolve_anchor(route, legacy_hint, legacy_opts).await
    }

    async fn is_visible(
        &self,
        route: ExecRoute,
        anchor: &mut AnchorDescriptor,
    ) -> Result<JudgeReport, PerceiverError>;

    async fn is_clickable(
        &self,
        route: ExecRoute,
        anchor: &mut AnchorDescriptor,
    ) -> Result<JudgeReport, PerceiverError>;

    async fn is_enabled(
        &self,
        route: ExecRoute,
        anchor: &mut AnchorDescriptor,
    ) -> Result<JudgeReport, PerceiverError>;

    async fn snapshot_dom_ax(&self, route: ExecRoute) -> Result<DomAxSnapshot, PerceiverError>;

    async fn snapshot_dom_ax_ext(
        &self,
        route: ExecRoute,
        scope: Scope,
        level: SnapLevel,
    ) -> Result<DomAxSnapshot, PerceiverError> {
        let _ = scope;
        let _ = level;
        self.snapshot_dom_ax(route).await
    }

    async fn diff_dom_ax(
        &self,
        route: ExecRoute,
        base: &DomAxSnapshot,
        current: &DomAxSnapshot,
    ) -> Result<DomAxDiff, PerceiverError>;

    async fn diff_dom_ax_ext(
        &self,
        route: ExecRoute,
        base: &DomAxSnapshot,
        current: &DomAxSnapshot,
        focus: Option<DiffFocus>,
    ) -> Result<DomAxDiff, PerceiverError> {
        let _ = focus;
        self.diff_dom_ax(route, base, current).await
    }

    fn advice_for_interaction(&self, _anchor: &AnchorDescriptor) -> Option<InteractionAdvice> {
        None
    }
}

impl From<&ResolveOpt> for ResolveOptions {
    fn from(value: &ResolveOpt) -> Self {
        ResolveOptions {
            max_candidates: if value.max_candidates == 0 {
                1
            } else {
                value.max_candidates
            },
            fuzziness: value.fuzziness,
            debounce_ms: value.debounce_ms,
        }
    }
}
