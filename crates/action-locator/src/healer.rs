//! Self-healing mechanism with one-time heal limit

use crate::{errors::LocatorError, resolver::ElementResolver, types::*};
use action_primitives::AnchorDescriptor;
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

/// Self-healer trait
#[async_trait]
pub trait SelfHealer: Send + Sync {
    /// Attempt to heal a failed anchor resolution
    async fn heal(&self, request: HealRequest) -> Result<HealOutcome, LocatorError>;

    /// Check if heal is available for this anchor
    fn is_heal_available(&self, anchor: &AnchorDescriptor) -> bool;

    /// Mark anchor as healed (consumes heal attempt)
    fn mark_healed(&self, anchor: &AnchorDescriptor);

    /// Reset heal history (for testing)
    fn reset(&self);
}

/// Default self-healer implementation
///
/// Enforces one-time heal limit per anchor using a hash set to track
/// which anchors have already been healed.
pub struct DefaultSelfHealer {
    resolver: Arc<dyn ElementResolver>,
    healed_anchors: Arc<Mutex<HashSet<String>>>,
}

impl DefaultSelfHealer {
    /// Create a new self-healer
    pub fn new(resolver: Arc<dyn ElementResolver>) -> Self {
        Self {
            resolver,
            healed_anchors: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Get anchor key for tracking
    fn anchor_key(anchor: &AnchorDescriptor) -> String {
        anchor.to_string()
    }

    /// Validate heal request
    fn validate_request(&self, request: &HealRequest) -> Result<(), LocatorError> {
        // Check if heal was already used
        if !self.is_heal_available(&request.original_anchor) {
            return Err(LocatorError::HealFailed(
                "Heal already used for this anchor".to_string(),
            ));
        }

        // Validate confidence threshold
        if request.min_confidence < 0.0 || request.min_confidence > 1.0 {
            return Err(LocatorError::HealFailed(format!(
                "Invalid confidence threshold: {}",
                request.min_confidence
            )));
        }

        // Validate max candidates
        if request.max_candidates == 0 {
            return Err(LocatorError::HealFailed(
                "max_candidates must be > 0".to_string(),
            ));
        }

        Ok(())
    }
}

#[async_trait]
impl SelfHealer for DefaultSelfHealer {
    async fn heal(&self, request: HealRequest) -> Result<HealOutcome, LocatorError> {
        info!(
            "Attempting self-heal for anchor: {}",
            request.original_anchor.to_string()
        );

        // Validate request
        if let Err(e) = self.validate_request(&request) {
            warn!("Heal validation failed: {}", e);
            return Ok(HealOutcome::Skipped {
                reason: e.to_string(),
            });
        }

        // Generate fallback plan
        debug!("Generating fallback plan");
        let plan = self
            .resolver
            .generate_fallback_plan(&request.original_anchor, &request.route)
            .await?;

        if !plan.has_fallbacks {
            warn!("No fallback candidates found");
            return Ok(HealOutcome::Exhausted {
                candidates: Vec::new(),
            });
        }

        // Filter candidates by confidence threshold
        let mut acceptable = plan
            .fallbacks
            .into_iter()
            .filter(|c| c.confidence >= request.min_confidence)
            .collect::<Vec<_>>();

        if acceptable.is_empty() {
            warn!("No candidates meet confidence threshold");
            return Ok(HealOutcome::Exhausted {
                candidates: Vec::new(),
            });
        }

        // Sort by confidence and limit to max_candidates
        acceptable.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        acceptable.truncate(request.max_candidates);

        // Try each candidate in order of confidence
        for candidate in &acceptable {
            debug!(
                "Trying candidate: {} (confidence: {:.2}, strategy: {})",
                candidate.element_id,
                candidate.confidence,
                candidate.strategy.name()
            );

            // Attempt resolution with this candidate's anchor
            match self
                .resolver
                .resolve(&candidate.anchor, &request.route)
                .await
            {
                Ok(result) => {
                    info!(
                        "Heal successful with {} strategy (confidence: {:.2})",
                        result.strategy.name(),
                        result.confidence
                    );

                    // Mark this anchor as healed
                    self.mark_healed(&request.original_anchor);

                    return Ok(HealOutcome::Healed {
                        used_anchor: candidate.anchor.clone(),
                        confidence: result.confidence,
                        strategy: result.strategy,
                    });
                }
                Err(e) => {
                    debug!("Candidate failed: {}", e);
                    continue;
                }
            }
        }

        // All candidates exhausted
        warn!("All {} candidates exhausted", acceptable.len());
        Ok(HealOutcome::Exhausted {
            candidates: acceptable,
        })
    }

    fn is_heal_available(&self, anchor: &AnchorDescriptor) -> bool {
        let key = Self::anchor_key(anchor);
        let healed = self.healed_anchors.lock().unwrap();
        !healed.contains(&key)
    }

    fn mark_healed(&self, anchor: &AnchorDescriptor) {
        let key = Self::anchor_key(anchor);
        let mut healed = self.healed_anchors.lock().unwrap();
        healed.insert(key);
    }

    fn reset(&self) {
        let mut healed = self.healed_anchors.lock().unwrap();
        healed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anchor_key() {
        let anchor = AnchorDescriptor::Css("#button".to_string());
        let key = DefaultSelfHealer::anchor_key(&anchor);
        assert_eq!(key, "css:#button");
    }

    #[test]
    fn test_heal_outcome() {
        let outcome = HealOutcome::Healed {
            used_anchor: AnchorDescriptor::Css("#submit".to_string()),
            confidence: 0.9,
            strategy: LocatorStrategy::Css,
        };

        assert!(outcome.is_success());
        assert!(outcome.healed_anchor().is_some());
        assert_eq!(outcome.confidence(), Some(0.9));
    }

    #[test]
    fn test_heal_outcome_failure() {
        let outcome = HealOutcome::Skipped {
            reason: "Already used".to_string(),
        };

        assert!(!outcome.is_success());
        assert!(outcome.healed_anchor().is_none());
        assert!(outcome.confidence().is_none());
    }

    #[test]
    fn test_candidate_confidence_checks() {
        let candidate = Candidate::new(
            "elem1".to_string(),
            LocatorStrategy::Css,
            0.85,
            AnchorDescriptor::Css("#button".to_string()),
        );

        assert!(candidate.is_high_confidence());
        assert!(candidate.is_acceptable());

        let low_confidence = Candidate::new(
            "elem2".to_string(),
            LocatorStrategy::Text,
            0.4,
            AnchorDescriptor::Text {
                content: "Click".to_string(),
                exact: false,
            },
        );

        assert!(!low_confidence.is_high_confidence());
        assert!(!low_confidence.is_acceptable());
    }
}
