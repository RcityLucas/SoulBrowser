use soulbrowser_core_types::ActionId;

use crate::model::ValueDigest;
use crate::ports::PrecheckEvent;

#[derive(Clone, Debug, Default)]
pub struct TypeEvents;

impl TypeEvents {
    pub async fn emit_started(&self, _action: &ActionId) {}
    pub async fn emit_precheck(&self, _action: &ActionId, _pre: &PrecheckEvent) {}
    pub async fn emit_finished(&self, _action: &ActionId, _value: &ValueDigest, _ok: bool) {}
}
