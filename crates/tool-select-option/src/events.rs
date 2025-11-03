use soulbrowser_core_types::ActionId;

use crate::ports::{PostEventPayload, PrecheckEvent};

#[derive(Clone, Debug, Default)]
pub struct SelectEvents;

impl SelectEvents {
    pub async fn emit_started(&self, _action: &ActionId) {}
    pub async fn emit_precheck(&self, _action: &ActionId, _snapshot: &PrecheckEvent) {}
    pub async fn emit_finished(&self, _action: &ActionId, _payload: &PostEventPayload, _ok: bool) {}
}
