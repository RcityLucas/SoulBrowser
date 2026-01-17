//! Click primitive - Click element with fallback strategies

use crate::{
    errors::ActionError,
    locator::apply_resolution_metadata,
    primitives::DefaultActionPrimitives,
    types::{ActionReport, AnchorDescriptor, ExecCtx, PostSignals, WaitTier},
};
use cdp_adapter::{AdapterErrorKind, Cdp};
use chrono::Utc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Execute click primitive
///
/// Clicks the element identified by anchor with built-in waiting.
/// Default tier: DomReady (wait for DOM to stabilize after click)
///
/// Steps:
/// 1. Validate anchor and context
/// 2. Resolve element via locator (with fallback)
/// 3. Check element is clickable (visible, enabled, not obscured)
/// 4. Execute CDP click command
/// 5. Apply built-in waiting based on tier
/// 6. Capture post-signals
/// 7. Generate action report
pub async fn execute_click(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    anchor: &AnchorDescriptor,
    wait_tier: WaitTier,
) -> Result<ActionReport, ActionError> {
    let started_at = Utc::now();
    let start_instant = Instant::now();

    info!(
        action_id = %ctx.action_id,
        anchor = %anchor.to_string(),
        wait_tier = ?wait_tier,
        "Executing click primitive"
    );

    // 1. Check context
    if ctx.is_cancelled() {
        return Err(ActionError::Interrupted("Context cancelled".to_string()));
    }

    if ctx.is_timeout() {
        return Err(ActionError::WaitTimeout(
            "Context deadline exceeded".to_string(),
        ));
    }

    // 2. Resolve selector
    debug!("Resolving element via anchor: {}", anchor.to_string());
    let resolved = primitives.resolve_anchor_selector(ctx, anchor).await?;
    let selector = resolved.selector.clone();
    let context = resolved.context.clone();

    // 3. Execute CDP click
    debug!("Executing CDP click");
    primitives
        .adapter()
        .click_in_context(&context, &selector, ctx.remaining_time())
        .await
        .map_err(|err| {
            let hint = err.hint.clone();
            let message = err.to_string();
            match err.kind {
                AdapterErrorKind::TargetNotFound => {
                    let detail = hint.unwrap_or_else(|| {
                        format!("Click target not found for selector '{}'", selector)
                    });
                    ActionError::AnchorNotFound(detail)
                }
                AdapterErrorKind::OptionNotFound => {
                    let detail = hint.unwrap_or_else(|| "Option not found".to_string());
                    ActionError::OptionNotFound(detail)
                }
                AdapterErrorKind::NavTimeout => {
                    let detail = hint.unwrap_or(message.clone());
                    ActionError::WaitTimeout(detail)
                }
                AdapterErrorKind::PolicyDenied => ActionError::PolicyDenied(message),
                AdapterErrorKind::CdpIo | AdapterErrorKind::Internal => ActionError::CdpIo(message),
            }
        })?;

    // 5. Apply built-in waiting
    if wait_tier != WaitTier::None {
        debug!("Applying built-in wait tier: {:?}", wait_tier);
        primitives
            .wait_strategy()
            .wait(primitives.adapter().clone(), context.page, wait_tier)
            .await?;
    }

    // 6. Capture post-signals
    let post_signals = capture_post_signals(primitives, ctx).await;

    // 7. Generate report
    let latency_ms = start_instant.elapsed().as_millis() as u64;

    info!(
        action_id = %ctx.action_id,
        latency_ms = latency_ms,
        "Click completed successfully"
    );

    let report = ActionReport::success(started_at, latency_ms).with_signals(post_signals);
    Ok(apply_resolution_metadata(report, &resolved))
}

/// Capture post-click signals
async fn capture_post_signals(primitives: &DefaultActionPrimitives, ctx: &ExecCtx) -> PostSignals {
    match primitives.capture_page_signals(ctx).await {
        Ok(signals) => signals,
        Err(err) => {
            warn!("failed to capture click signals: {}", err);
            PostSignals::default()
        }
    }
}
