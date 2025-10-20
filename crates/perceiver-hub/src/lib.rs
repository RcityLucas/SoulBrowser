///! Perception Hub - Multi-Modal Perception System
///!
///! This crate provides a unified interface for coordinating multiple perception
///! modalities (Structural, Visual, and Semantic) to enable comprehensive page
///! understanding for browser automation.
///!
///! # Architecture
///!
///! The Perception Hub integrates three specialized perceivers:
///!
///! - **Structural Perceiver**: DOM/AX tree analysis, element resolution
///! - **Visual Perceiver**: Screenshot analysis, visual metrics, OCR
///! - **Semantic Perceiver**: Content classification, text analysis, intent extraction
///!
///! # Example
///!
///! ```rust,no_run
///! use perceiver_hub::{PerceptionHub, PerceptionHubImpl, PerceptionOptions};
///! use std::sync::Arc;
///!
///! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
///! // Create perceivers (simplified)
///! # let structural = unimplemented!();
///! # let visual = unimplemented!();
///! # let semantic = unimplemented!();
///!
///! // Create hub
///! let hub = PerceptionHubImpl::new(
///!     Arc::new(structural),
///!     Arc::new(visual),
///!     Arc::new(semantic),
///! );
///!
///! // Perform multi-modal analysis
///! # let route = unimplemented!();
///! let perception = hub.perceive(&route, PerceptionOptions::default()).await?;
///!
///! println!("Confidence: {:.2}", perception.confidence);
///! println!("Content type: {:?}", perception.semantic.as_ref().map(|s| &s.content_type));
///! println!("Insights: {} found", perception.insights.len());
///! # Ok(())
///! # }
///! ```

pub mod errors;
pub mod hub;
pub mod models;

// Re-exports
pub use errors::{HubError, Result};
pub use hub::{PerceptionHub, PerceptionHubImpl};
pub use models::*;

// Re-export perceiver types for convenience
pub use perceiver_semantic::{SemanticPerceiver, SemanticPerceiverImpl};
pub use perceiver_structural::{StructuralPerceiver, StructuralPerceiverImpl};
pub use perceiver_visual::{VisualPerceiver, VisualPerceiverImpl};
