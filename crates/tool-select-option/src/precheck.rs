use std::time::Instant;

use perceiver_structural::AnchorDescriptor;
use soulbrowser_core_types::{ExecRoute, SoulError};
use tracing::warn;

use crate::model::FieldSnapshot;
use crate::policy::SelectTimeouts;
use crate::ports::{CdpPort, StructPort};

pub async fn run_precheck(
    struct_port: &dyn StructPort,
    cdp: &dyn CdpPort,
    route: &ExecRoute,
    anchor: &AnchorDescriptor,
    timeouts: &SelectTimeouts,
) -> Result<FieldSnapshot, SoulError> {
    let start = Instant::now();
    let mut visible = struct_port.is_visible(route, anchor).await?;
    if !visible {
        cdp.scroll_into_view(route, anchor).await?;
        visible = struct_port.is_visible(route, anchor).await?;
    }
    let clickable = struct_port.is_clickable(route, anchor).await?;
    let enabled = struct_port.is_enabled(route, anchor).await?;
    let readonly = struct_port.is_readonly(route, anchor).await?;

    if clickable {
        if let Err(err) = cdp.focus(route, anchor).await {
            warn!("select-option focus warn: {}", err);
        }
    }

    if start.elapsed() > timeouts.precheck() {
        warn!("select-option precheck exceeded timeout");
    }

    Ok(FieldSnapshot {
        visible,
        clickable,
        enabled,
        readonly,
    })
}
