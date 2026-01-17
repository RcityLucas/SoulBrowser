//! L3 Locator & Self-heal - Multi-strategy element resolution
//!
//! This crate implements the intelligent element location system with:
//! - CSS selector resolution (primary strategy)
//! - ARIA/AX fallback (accessibility-based)
//! - Text content fallback (semantic matching)
//! - One-time self-heal mechanism with confidence scoring
//! - Candidate ranking and selection

pub mod bridge;
pub mod errors;
pub mod healer;
pub mod resolver;
pub mod strategies;
pub mod types;

pub use bridge::*;
pub use errors::*;
pub use healer::*;
pub use resolver::*;
pub use strategies::*;
pub use types::*;

/// Returns `true` when the locator is compiled in stub mode.
pub const fn is_stubbed() -> bool {
    cfg!(feature = "stub")
}
