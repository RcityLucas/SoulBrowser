use chrono::{Duration as ChronoDuration, Utc};

use crate::model::WaitTier;
use crate::policy::ClickTimeouts;
use crate::ports::CdpPort;
use soulbrowser_core_types::{ExecRoute, SoulError};

pub async fn apply_wait(
    cdp: &dyn CdpPort,
    route: &ExecRoute,
    tier: WaitTier,
    timeouts: &ClickTimeouts,
) -> Result<(), SoulError> {
    match tier {
        WaitTier::None => Ok(()),
        WaitTier::Auto | WaitTier::DomReady => {
            let deadline = Utc::now()
                + ChronoDuration::from_std(timeouts.wait_for(tier))
                    .unwrap_or_else(|_| ChronoDuration::seconds(1));
            cdp.wait_dom_ready(route, deadline).await
        }
    }
}
