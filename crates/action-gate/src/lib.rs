//! L3 Post-conditions Gate - Multi-signal validation
//!
//! This crate implements the post-action validation system with:
//! - ExpectSpec rule model (all/any/deny conditions)
//! - Multi-signal validation (DOM/Network/URL/Title/Runtime)
//! - Evidence collection for validation results
//! - Locator hints for suspicious cases
//! - Integration with action primitives

pub mod conditions;
pub mod errors;
pub mod evidence;
pub mod types;
pub mod validator;

pub use conditions::*;
pub use errors::*;
pub use evidence::*;
pub use types::*;
pub use validator::*;
