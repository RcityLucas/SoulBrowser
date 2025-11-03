#[derive(Clone, Debug, Default)]
pub struct SelectMetrics;

impl SelectMetrics {
    pub fn record_ok(&self, _latency: u128) {}
    pub fn record_fail(&self, _kind: &str) {}
    pub fn record_precheck_failure(&self, _field: &str) {}
    pub fn record_self_heal(&self, _success: bool) {}
    pub fn record_mode(&self, _mode: &str) {}
    pub fn record_match_kind(&self, _kind: &str) {}
}
