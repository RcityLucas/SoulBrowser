use std::time::Instant;

use perceiver_structural::AnchorDescriptor;
use soulbrowser_core_types::{ExecRoute, SoulError};
use tracing::warn;

use crate::model::FieldSnapshot;
use crate::policy::TypeTimeouts;
use crate::ports::{CdpPort, StructPort};

pub async fn run_precheck(
    struct_port: &dyn StructPort,
    cdp: &dyn CdpPort,
    route: &ExecRoute,
    anchor: &AnchorDescriptor,
    timeouts: &TypeTimeouts,
) -> Result<FieldSnapshot, SoulError> {
    let start = Instant::now();
    let mut visible = struct_port.is_visible(route, anchor).await?;
    if !visible {
        cdp.scroll_into_view(route, anchor).await?;
        visible = struct_port.is_visible(route, anchor).await?;
    }
    let clickable = struct_port.is_clickable(route, anchor).await?;
    let enabled = struct_port.is_enabled(route, anchor).await?;
    let meta = struct_port.field_meta(route, anchor).await?;
    if clickable {
        if let Err(err) = cdp.focus(route, anchor).await {
            warn!("type-text focus warn: {}", err);
        }
    }
    if start.elapsed() > timeouts.precheck() {
        warn!("type-text precheck exceeded timeout");
    }
    Ok(FieldSnapshot {
        visible,
        clickable,
        enabled,
        readonly: meta.readonly,
        maxlength: meta.maxlength,
        password_like: meta.password_like,
    })
}
