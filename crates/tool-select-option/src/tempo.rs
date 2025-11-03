use async_trait::async_trait;
use tokio::time::{sleep, Duration};

use crate::model::SelectMode;
use crate::ports::{TempoPlan, TempoPort};
use perceiver_structural::AnchorDescriptor;
use soulbrowser_core_types::{ExecRoute, SoulError};

/// No-op tempo helper; real implementations may introduce human-like pacing.
#[derive(Clone, Debug, Default)]
pub struct NullTempo;

#[async_trait]
impl TempoPort for NullTempo {
    async fn plan(
        &self,
        _route: &ExecRoute,
        _control: &AnchorDescriptor,
        _mode: SelectMode,
    ) -> Result<TempoPlan, SoulError> {
        Ok(TempoPlan::default())
    }

    async fn apply(&self, plan: &TempoPlan) -> Result<(), SoulError> {
        if plan.pre_delay_ms > 0 {
            sleep(Duration::from_millis(plan.pre_delay_ms)).await;
        }
        if plan.post_delay_ms > 0 {
            sleep(Duration::from_millis(plan.post_delay_ms)).await;
        }
        Ok(())
    }
}
