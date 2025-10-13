#![allow(dead_code)]

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("not found")]
    NotFound,
    #[error("route stale")]
    RouteStale,
    #[error("ownership conflict")]
    OwnershipConflict,
    #[error("limit reached")]
    LimitReached,
    #[error("internal error")]
    Internal,
}

impl RegistryError {
    pub fn into_soul_error(self, detail: impl Into<String>) -> soulbrowser_core_types::SoulError {
        let message = format!("{}: {}", self, detail.into());
        soulbrowser_core_types::SoulError::new(message)
    }
}
