use crate::model::WaitTier;
use crate::policy::SelectTimeouts;
use crate::ports::CdpPort;
use soulbrowser_core_types::{ExecRoute, SoulError};

pub async fn apply_wait(
    cdp: &dyn CdpPort,
    route: &ExecRoute,
    tier: WaitTier,
    timeouts: &SelectTimeouts,
) -> Result<(), SoulError> {
    match tier {
        WaitTier::None => Ok(()),
        WaitTier::Auto | WaitTier::DomReady => {
            cdp.wait_dom_ready(route, timeouts.wait_for(tier)).await
        }
    }
}
