//! Placeholder for future plugin adapter wiring.

use crate::errors::{AdapterError, AdapterResult};

/// Registers plugin surfaces with the adapter.
///
/// The plugin runtime is not yet wired; this function simply reports that the
/// feature is unavailable so downstream callers can handle the capability
/// check gracefully.
pub fn register_plugins() -> AdapterResult<()> {
    Err(AdapterError::NotImplemented(
        "plugin surfaces are not yet implemented",
    ))
}
