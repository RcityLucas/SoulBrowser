//! Element resolver with fallback chain orchestration

use crate::{errors::LocatorError, strategies::*, types::*};
use action_primitives::AnchorDescriptor;
use async_trait::async_trait;
use cdp_adapter::CdpAdapter;
use perceiver_structural::StructuralPerceiver;
use soulbrowser_core_types::ExecRoute;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Element resolver trait
#[async_trait]
pub trait ElementResolver: Send + Sync {
    /// Resolve element with fallback chain
    async fn resolve(
        &self,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<ResolutionResult, LocatorError>;

    /// Generate fallback plan for an anchor
    async fn generate_fallback_plan(
        &self,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<FallbackPlan, LocatorError>;

    /// Try to resolve with specific strategy
    async fn resolve_with_strategy(
        &self,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
        strategy: LocatorStrategy,
    ) -> Result<Vec<Candidate>, LocatorError>;
}

/// Default element resolver implementation
pub struct DefaultElementResolver {
    css_strategy: Arc<CssStrategy>,
    aria_strategy: Arc<AriaAxStrategy>,
    text_strategy: Arc<TextStrategy>,
}

impl DefaultElementResolver {
    /// Create a new resolver with all strategies
    pub fn new(adapter: Arc<CdpAdapter>, perceiver: Arc<dyn StructuralPerceiver>) -> Self {
        Self {
            css_strategy: Arc::new(CssStrategy::new(adapter.clone(), perceiver.clone())),
            aria_strategy: Arc::new(AriaAxStrategy::new(perceiver.clone())),
            text_strategy: Arc::new(TextStrategy::new(perceiver.clone())),
        }
    }

    /// Get strategy by type
    fn get_strategy(&self, strategy_type: LocatorStrategy) -> Arc<dyn Strategy> {
        match strategy_type {
            LocatorStrategy::Css => self.css_strategy.clone(),
            LocatorStrategy::AriaAx => self.aria_strategy.clone(),
            LocatorStrategy::Text => self.text_strategy.clone(),
        }
    }
}

#[async_trait]
impl ElementResolver for DefaultElementResolver {
    async fn resolve(
        &self,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<ResolutionResult, LocatorError> {
        info!("Resolving element: {}", anchor.to_string());

        // Try each strategy in fallback order
        for strategy_type in LocatorStrategy::fallback_chain() {
            debug!("Trying strategy: {}", strategy_type.name());

            let strategy = self.get_strategy(strategy_type.clone());

            match strategy.resolve(anchor, route).await {
                Ok(candidates) if !candidates.is_empty() => {
                    // Found candidates, select the best one
                    let best = select_best_candidate(&candidates)?;

                    info!(
                        "Resolved element using {} strategy: {} (confidence: {:.2})",
                        strategy_type.name(),
                        best.element_id,
                        best.confidence
                    );

                    return Ok(ResolutionResult::new(
                        best.element_id.clone(),
                        strategy_type,
                        best.confidence,
                        best.anchor.clone(),
                    ));
                }
                Ok(_) => {
                    debug!("Strategy {} returned no candidates", strategy_type.name());
                }
                Err(e) => {
                    warn!("Strategy {} failed: {}", strategy_type.name(), e);
                }
            }
        }

        // All strategies failed
        Err(LocatorError::ElementNotFound(format!(
            "All strategies exhausted for anchor: {}",
            anchor.to_string()
        )))
    }

    async fn generate_fallback_plan(
        &self,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<FallbackPlan, LocatorError> {
        let mut plan = FallbackPlan::new(anchor.clone());

        // Try each strategy and collect all candidates
        for strategy_type in LocatorStrategy::fallback_chain() {
            let strategy = self.get_strategy(strategy_type);

            match strategy.resolve(anchor, route).await {
                Ok(candidates) => {
                    for candidate in candidates {
                        plan.add_fallback(candidate);
                    }
                }
                Err(e) => {
                    debug!(
                        "Strategy {} failed during plan generation: {}",
                        strategy_type.name(),
                        e
                    );
                }
            }
        }

        // Sort candidates by confidence
        plan.fallbacks.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(plan)
    }

    async fn resolve_with_strategy(
        &self,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
        strategy_type: LocatorStrategy,
    ) -> Result<Vec<Candidate>, LocatorError> {
        let strategy = self.get_strategy(strategy_type);
        strategy.resolve(anchor, route).await
    }
}

/// Select best candidate from a list
fn select_best_candidate(candidates: &[Candidate]) -> Result<&Candidate, LocatorError> {
    if candidates.is_empty() {
        return Err(LocatorError::ElementNotFound(
            "No candidates provided".to_string(),
        ));
    }

    // Find candidate with highest confidence
    let best = candidates
        .iter()
        .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap())
        .unwrap(); // Safe because we checked !is_empty()

    // Check if we have multiple high-confidence candidates (ambiguous)
    let high_confidence_count = candidates.iter().filter(|c| c.is_high_confidence()).count();

    if high_confidence_count > 1 {
        warn!(
            "Ambiguous match: {} high-confidence candidates found",
            high_confidence_count
        );
        // Still return best, but log warning
    }

    // Check if best candidate is acceptable
    if !best.is_acceptable() {
        return Err(LocatorError::ElementNotFound(format!(
            "Best candidate has low confidence: {:.2}",
            best.confidence
        )));
    }

    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_best_candidate() {
        let candidates = vec![
            Candidate::new(
                "elem1".to_string(),
                LocatorStrategy::Css,
                0.9,
                AnchorDescriptor::Css("#button".to_string()),
            ),
            Candidate::new(
                "elem2".to_string(),
                LocatorStrategy::AriaAx,
                0.7,
                AnchorDescriptor::Aria {
                    role: "button".to_string(),
                    name: "Submit".to_string(),
                },
            ),
        ];

        let best = select_best_candidate(&candidates).unwrap();
        assert_eq!(best.element_id, "elem1");
        assert_eq!(best.confidence, 0.9);
    }

    #[test]
    fn test_select_best_candidate_low_confidence() {
        let candidates = vec![Candidate::new(
            "elem1".to_string(),
            LocatorStrategy::Css,
            0.3, // Below threshold
            AnchorDescriptor::Css("#button".to_string()),
        )];

        let result = select_best_candidate(&candidates);
        assert!(result.is_err());
    }

    #[test]
    fn test_select_best_candidate_empty() {
        let candidates: Vec<Candidate> = vec![];
        let result = select_best_candidate(&candidates);
        assert!(result.is_err());
    }
}
