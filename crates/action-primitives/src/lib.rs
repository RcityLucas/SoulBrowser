//! L3 Action Primitives - Core browser automation operations
//!
//! This crate provides the fundamental building blocks for intelligent browser automation:
//! - 6 core primitives: navigate, click, type, select, scroll, wait
//! - Built-in waiting with DomReady and Idle tiers
//! - Comprehensive error handling and reporting
//! - Integration with CDP adapter and perception layers

pub mod errors;
mod locator;
mod primitives;
pub mod types;
mod waiting;

pub use errors::*;
pub use locator::*;
pub use primitives::*;
pub use types::*;
pub use waiting::*;
