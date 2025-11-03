#![allow(dead_code)]

pub mod api;
pub mod config;
pub mod errors;
pub mod metrics;
pub mod model;

pub mod cold;
pub mod hot;
pub mod read;

mod drop_policy;
mod idempotency;
mod redact;

pub use api::{EventStore, EventStoreBuilder, PostHook};
pub use config::EsPolicyView;
pub use errors::{EsError, EsErrorKind};
pub use model::StreamBatch;
pub use read::stream::EventStreamCursor;
