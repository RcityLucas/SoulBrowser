use std::time::Instant;

use perceiver_structural::AnchorDescriptor;
use soulbrowser_core_types::{ExecRoute, SoulError};
use tracing::warn;

use crate::model::PrecheckSnapshot;
use crate::policy::ClickTimeouts;
use crate::ports::{CdpPort, StructPort};

pub async fn run_precheck(
    struct_port: &dyn StructPort,
    cdp: &dyn CdpPort,
    route: &ExecRoute,
    anchor: &AnchorDescriptor,
    timeouts: &ClickTimeouts,
) -> Result<PrecheckSnapshot, SoulError> {
    let start = Instant::now();
    let mut visible = struct_port.is_visible(route, anchor).await?;
    if !visible {
        cdp.scroll_into_view(route, anchor).await?;
        visible = struct_port.is_visible(route, anchor).await?;
    }
    let clickable = struct_port.is_clickable(route, anchor).await?;
    let enabled = struct_port.is_enabled(route, anchor).await?;

    if clickable {
        if let Err(err) = cdp.focus(route, anchor).await {
            warn!("click precheck focus failed: {}", err);
        }
    }

    if start.elapsed() > timeouts.precheck() {
        warn!("click precheck exceeded timeout");
    }

    Ok(PrecheckSnapshot {
        visible,
        clickable,
        enabled,
    })
}
