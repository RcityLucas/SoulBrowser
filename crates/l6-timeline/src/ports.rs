use crate::errors::TlError;
use crate::model::{EventEnvelope, QueryDigest, ReplayBundle};
use crate::policy::TimelinePolicyView;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

#[async_trait]
pub trait EventStorePort: Send + Sync {
    async fn by_action(&self, action_id: &str) -> Result<Vec<EventEnvelope>, TlError>;
    async fn by_flow_window(&self, flow_id: &str) -> Result<Vec<EventEnvelope>, TlError>;
    async fn by_task_window(&self, task_id: &str) -> Result<Vec<EventEnvelope>, TlError>;
    async fn export_range(
        &self,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<Vec<EventEnvelope>, TlError>;
    async fn replay_minimal(&self, action_id: &str) -> Result<ReplayBundle, TlError>;
    async fn hot_window_hint(&self) -> Result<(DateTime<Utc>, DateTime<Utc>), TlError>;
}

#[async_trait]
pub trait StateCenterPort: Send + Sync {
    async fn tail(&self, limit: usize) -> Result<Vec<EventEnvelope>, TlError>;
}

pub trait PolicyPort: Send + Sync {
    fn view(&self) -> TimelinePolicyView;
}

pub trait EventsPort: Send + Sync {
    fn timeline_export_started(&self, digest: &QueryDigest);
    fn timeline_export_fetched(&self, count: usize, source: &str);
    fn timeline_export_finished(&self, ok: bool, latency_ms: u128, err: Option<&TlError>);
}
