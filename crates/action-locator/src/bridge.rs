use std::sync::Arc;

use async_trait::async_trait;
use tracing::warn;

use crate::{
    errors::LocatorError,
    healer::{DefaultSelfHealer, SelfHealer},
    resolver::{DefaultElementResolver, ElementResolver},
    types::{HealOutcome, HealRequest, ResolutionResult},
};
use action_primitives::{
    errors::ActionError,
    types::{AnchorDescriptor, ExecCtx, SelfHealInfo},
    AnchorResolver, DefaultActionPrimitives, ResolvedSelector, ScriptAnchorResolver,
};
use perceiver_structural::StructuralPerceiver;

/// Bridge implementation that adapts `action-locator` to the action primitive resolver trait.
pub struct LocatorBackedResolver {
    resolver: Arc<dyn ElementResolver>,
    healer: Option<Arc<dyn SelfHealer>>,
    script: ScriptAnchorResolver,
}

impl LocatorBackedResolver {
    /// Create a resolver from arbitrary locator/healer implementations.
    pub fn new(resolver: Arc<dyn ElementResolver>, healer: Option<Arc<dyn SelfHealer>>) -> Self {
        Self {
            resolver,
            healer,
            script: ScriptAnchorResolver::default(),
        }
    }

    /// Convenience helper that wires the default element resolver + self-healer.
    pub fn with_default(
        adapter: Arc<cdp_adapter::CdpAdapter>,
        perceiver: Arc<dyn StructuralPerceiver>,
    ) -> Self {
        let resolver = Arc::new(DefaultElementResolver::new(adapter, perceiver));
        let healer = Arc::new(DefaultSelfHealer::new(resolver.clone()));
        if crate::is_stubbed() {
            warn!("Action locator built in stub mode; no real selectors will be resolved");
        }
        Self::new(resolver, Some(healer))
    }

    async fn resolve_with_result(
        &self,
        primitives: &DefaultActionPrimitives,
        ctx: &ExecCtx,
        result: ResolutionResult,
        heal_info: Option<SelfHealInfo>,
    ) -> Result<ResolvedSelector, ActionError> {
        let mut resolved = self.script.resolve(primitives, ctx, &result.anchor).await?;
        resolved.strategy = Some(result.strategy.name().to_string());
        resolved.confidence = Some(result.confidence);
        resolved.heal_info = heal_info;
        Ok(resolved)
    }

    async fn attempt_heal(
        &self,
        primitives: &DefaultActionPrimitives,
        ctx: &ExecCtx,
        anchor: &AnchorDescriptor,
    ) -> Result<ResolvedSelector, ActionError> {
        let healer = match &self.healer {
            Some(healer) => healer,
            None => return self.script.resolve(primitives, ctx, anchor).await,
        };

        let request = HealRequest::new(anchor.clone(), ctx.route.clone());
        match healer.heal(request).await {
            Ok(HealOutcome::Healed {
                used_anchor,
                confidence,
                strategy,
            }) => {
                let heal_info = SelfHealInfo {
                    original_anchor: anchor.to_string(),
                    healed_anchor: used_anchor.to_string(),
                    strategy: strategy.name().to_string(),
                    confidence,
                };
                let result = self
                    .resolver
                    .resolve(&used_anchor, &ctx.route)
                    .await
                    .map_err(map_locator_error)?
                    .with_heal();
                self.resolve_with_result(primitives, ctx, result, Some(heal_info))
                    .await
            }
            Ok(HealOutcome::Skipped { reason }) => Err(ActionError::AnchorNotFound(reason)),
            Ok(HealOutcome::Exhausted { .. }) => Err(ActionError::AnchorNotFound(
                "All healer candidates exhausted".to_string(),
            )),
            Ok(HealOutcome::Aborted { reason }) => Err(ActionError::Internal(reason)),
            Err(err) => Err(map_locator_error(err)),
        }
    }
}

#[async_trait]
impl AnchorResolver for LocatorBackedResolver {
    async fn resolve(
        &self,
        primitives: &DefaultActionPrimitives,
        ctx: &ExecCtx,
        anchor: &AnchorDescriptor,
    ) -> Result<ResolvedSelector, ActionError> {
        match self.resolver.resolve(anchor, &ctx.route).await {
            Ok(result) => {
                self.resolve_with_result(primitives, ctx, result, None)
                    .await
            }
            Err(err) => {
                if err.is_retryable() {
                    warn!("locator error: {}", err);
                }
                self.attempt_heal(primitives, ctx, anchor).await
            }
        }
    }
}

fn map_locator_error(err: LocatorError) -> ActionError {
    match err {
        LocatorError::ElementNotFound(reason)
        | LocatorError::AmbiguousMatch(reason)
        | LocatorError::InvalidAnchor(reason)
        | LocatorError::HealFailed(reason) => ActionError::AnchorNotFound(reason),
        LocatorError::CdpError(reason) => ActionError::CdpIo(reason),
        LocatorError::Timeout(reason) => ActionError::WaitTimeout(reason),
        LocatorError::StrategyFailed { reason, .. } => ActionError::Internal(reason),
        LocatorError::Internal(reason) => ActionError::Internal(reason),
    }
}
