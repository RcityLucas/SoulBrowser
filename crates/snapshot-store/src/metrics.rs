#[derive(Clone, Debug, Default)]
pub struct SnapMetrics;

impl SnapMetrics {
    pub fn record_put_struct(&self, _bytes: usize, _masked: bool) {}
    pub fn record_put_clip(&self, _bytes: usize) {}
    pub fn record_bind(&self) {}
    pub fn record_sweep(&self, _removed_struct: usize, _removed_pix: usize) {}
    pub fn record_warn(&self, _reason: &str) {}
}
