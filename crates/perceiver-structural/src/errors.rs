use thiserror::Error;

#[derive(Debug, Error)]
pub enum PerceiverError {
    #[error("anchor not found: {0}")]
    AnchorNotFound(String),
    #[error("snapshot stale")]
    SnapshotStale,
    #[error("frame lost")]
    FrameLost,
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl PerceiverError {
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}
