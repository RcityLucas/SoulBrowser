use soulbrowser_core_types::ActionId;

use crate::model::PostSignals;
use crate::ports::PrecheckEvent;

#[derive(Clone, Debug, Default)]
pub struct ClickEvents;

impl ClickEvents {
    pub async fn emit_started(&self, _action: &ActionId) {}
    pub async fn emit_precheck(&self, _action: &ActionId, _pre: &PrecheckEvent) {}
    pub async fn emit_finished(&self, _action: &ActionId, _signals: &PostSignals, _ok: bool) {}
}
