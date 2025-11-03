use crate::ports::{TempoPlan, TempoPort};
use async_trait::async_trait;
use perceiver_structural::AnchorDescriptor;
use soulbrowser_core_types::{ExecRoute, SoulError};

/// No-op tempo helper when stealth is disabled.
#[derive(Clone, Debug, Default)]
pub struct NullTempo;

#[async_trait]
impl TempoPort for NullTempo {
    async fn prepare(
        &self,
        _route: &ExecRoute,
        _anchor: &AnchorDescriptor,
    ) -> Result<TempoPlan, SoulError> {
        Ok(TempoPlan::default())
    }

    async fn apply(&self, _plan: &TempoPlan) -> Result<(), SoulError> {
        Ok(())
    }
}
