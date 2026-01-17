//! Core types for locator system

use action_primitives::AnchorDescriptor;
use serde::{Deserialize, Serialize};

/// Locator strategy enumeration
///
/// Defines the three strategies for element location:
/// - CSS: Direct CSS selector matching
/// - AriaAx: ARIA role and accessible name matching
/// - Text: Text content matching (exact or partial)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocatorStrategy {
    /// CSS selector strategy
    Css,

    /// ARIA/AX attributes strategy
    AriaAx,

    /// Text content strategy
    Text,
}

impl LocatorStrategy {
    /// Get strategy name as string
    pub fn name(&self) -> &'static str {
        match self {
            LocatorStrategy::Css => "css",
            LocatorStrategy::AriaAx => "aria-ax",
            LocatorStrategy::Text => "text",
        }
    }

    /// Get all strategies in fallback order
    pub fn fallback_chain() -> Vec<LocatorStrategy> {
        vec![
            LocatorStrategy::Css,
            LocatorStrategy::AriaAx,
            LocatorStrategy::Text,
        ]
    }
}

/// Element candidate for locator resolution
///
/// Represents a potential element match with scoring information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    /// Element ID or node reference
    pub element_id: String,

    /// Strategy used to find this candidate
    pub strategy: LocatorStrategy,

    /// Confidence score (0.0-1.0)
    pub confidence: f64,

    /// Matching anchor descriptor
    pub anchor: AnchorDescriptor,

    /// Additional metadata about the match
    pub metadata: CandidateMetadata,
}

impl Candidate {
    /// Create a new candidate
    pub fn new(
        element_id: String,
        strategy: LocatorStrategy,
        confidence: f64,
        anchor: AnchorDescriptor,
    ) -> Self {
        Self {
            element_id,
            strategy,
            confidence,
            anchor,
            metadata: CandidateMetadata::default(),
        }
    }

    /// Check if this is a high-confidence match (>= 0.8)
    pub fn is_high_confidence(&self) -> bool {
        self.confidence >= 0.8
    }

    /// Check if this is an acceptable match (>= 0.5)
    pub fn is_acceptable(&self) -> bool {
        self.confidence >= 0.5
    }
}

/// Candidate metadata
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CandidateMetadata {
    /// Element tag name
    pub tag_name: Option<String>,

    /// Element visible text
    pub visible_text: Option<String>,

    /// Element ARIA role
    pub aria_role: Option<String>,

    /// Element ARIA label
    pub aria_label: Option<String>,

    /// Element position in DOM (for disambiguation)
    pub dom_index: Option<usize>,

    /// Whether element is visible
    pub is_visible: bool,

    /// Whether element is enabled
    pub is_enabled: bool,
}

/// Fallback plan containing primary anchor and fallback candidates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackPlan {
    /// Primary anchor descriptor (from user)
    pub primary: AnchorDescriptor,

    /// Ordered list of fallback candidates
    pub fallbacks: Vec<Candidate>,

    /// Whether fallbacks were generated
    pub has_fallbacks: bool,
}

impl FallbackPlan {
    /// Create a new fallback plan with primary anchor only
    pub fn new(primary: AnchorDescriptor) -> Self {
        Self {
            primary,
            fallbacks: Vec::new(),
            has_fallbacks: false,
        }
    }

    /// Add fallback candidate
    pub fn add_fallback(&mut self, candidate: Candidate) {
        self.fallbacks.push(candidate);
        self.has_fallbacks = true;
    }

    /// Get best fallback candidate (highest confidence)
    pub fn best_fallback(&self) -> Option<&Candidate> {
        self.fallbacks
            .iter()
            .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap())
    }

    /// Get acceptable fallback candidates (confidence >= 0.5)
    pub fn acceptable_fallbacks(&self) -> Vec<&Candidate> {
        self.fallbacks
            .iter()
            .filter(|c| c.is_acceptable())
            .collect()
    }
}

/// Heal request for self-healing mechanism
#[derive(Debug, Clone)]
pub struct HealRequest {
    /// Original anchor that failed
    pub original_anchor: AnchorDescriptor,

    /// Execution route context
    pub route: soulbrowser_core_types::ExecRoute,

    /// Maximum candidates to evaluate
    pub max_candidates: usize,

    /// Minimum confidence threshold
    pub min_confidence: f64,
}

impl HealRequest {
    /// Create a new heal request with defaults
    pub fn new(
        original_anchor: AnchorDescriptor,
        route: soulbrowser_core_types::ExecRoute,
    ) -> Self {
        Self {
            original_anchor,
            route,
            max_candidates: 10,
            min_confidence: 0.5,
        }
    }

    /// Set maximum candidates to evaluate
    pub fn with_max_candidates(mut self, max: usize) -> Self {
        self.max_candidates = max;
        self
    }

    /// Set minimum confidence threshold
    pub fn with_min_confidence(mut self, min: f64) -> Self {
        self.min_confidence = min;
        self
    }
}

/// Heal outcome enumeration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HealOutcome {
    /// Successfully healed with new anchor
    Healed {
        /// The new anchor that worked
        used_anchor: AnchorDescriptor,

        /// Confidence of the heal
        confidence: f64,

        /// Strategy that succeeded
        strategy: LocatorStrategy,
    },

    /// Healing skipped (e.g., already used once)
    Skipped {
        /// Reason for skipping
        reason: String,
    },

    /// Exhausted all candidates without success
    Exhausted {
        /// All candidates that were tried
        candidates: Vec<Candidate>,
    },

    /// Healing aborted (e.g., timeout, cancellation)
    Aborted {
        /// Reason for abortion
        reason: String,
    },
}

impl HealOutcome {
    /// Check if heal was successful
    pub fn is_success(&self) -> bool {
        matches!(self, HealOutcome::Healed { .. })
    }

    /// Get healed anchor if successful
    pub fn healed_anchor(&self) -> Option<&AnchorDescriptor> {
        match self {
            HealOutcome::Healed { used_anchor, .. } => Some(used_anchor),
            _ => None,
        }
    }

    /// Get confidence if successful
    pub fn confidence(&self) -> Option<f64> {
        match self {
            HealOutcome::Healed { confidence, .. } => Some(*confidence),
            _ => None,
        }
    }
}

/// Element resolution result
#[derive(Debug, Clone)]
pub struct ResolutionResult {
    /// Resolved element ID
    pub element_id: String,

    /// Strategy used for resolution
    pub strategy: LocatorStrategy,

    /// Confidence score
    pub confidence: f64,

    /// Whether this was from a heal attempt
    pub from_heal: bool,

    /// Anchor descriptor that matched
    pub anchor: AnchorDescriptor,
}

impl ResolutionResult {
    /// Create a new resolution result
    pub fn new(
        element_id: String,
        strategy: LocatorStrategy,
        confidence: f64,
        anchor: AnchorDescriptor,
    ) -> Self {
        Self {
            element_id,
            strategy,
            confidence,
            from_heal: false,
            anchor,
        }
    }

    /// Mark this result as from heal attempt
    pub fn with_heal(mut self) -> Self {
        self.from_heal = true;
        self
    }
}
