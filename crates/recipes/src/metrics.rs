#[derive(Clone, Debug, Default)]
pub struct RecMetrics;

impl RecMetrics {
    pub fn record_ingest(&self) {}
    pub fn record_suggest(&self, _rank: f32) {}
    pub fn record_promote(&self) {}
    pub fn record_rollback(&self) {}
    pub fn record_hygiene(&self) {}
    pub fn record_policy_reload(&self) {}
    pub fn record_governance(&self, _action: &str) {}
}
