#[derive(Clone, Debug, Default)]
pub struct ClickMetrics;

impl ClickMetrics {
    pub fn record_ok(&self, _latency: u128) {}
    pub fn record_fail(&self, _kind: &str) {}
    pub fn record_precheck_failure(&self, _field: &str) {}
    pub fn record_self_heal(&self, _success: bool) {}
}
