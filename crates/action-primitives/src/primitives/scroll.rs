//! Scroll primitive - Scroll page or element into view

use crate::{
    errors::ActionError,
    locator::apply_resolution_metadata,
    primitives::DefaultActionPrimitives,
    types::{ActionReport, ExecCtx, PostSignals, ScrollBehavior, ScrollTarget},
};
use cdp_adapter::{Cdp, ResolvedExecutionContext};
use chrono::Utc;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Execute scroll primitive
///
/// Scrolls to the specified target with smooth or instant behavior.
/// No built-in waiting (scroll is immediate).
///
/// Steps:
/// 1. Validate target and context
/// 2. Resolve target (if element-based)
/// 3. Calculate scroll position
/// 4. Execute CDP scroll command
/// 5. Wait for scroll completion (if smooth)
/// 6. Capture post-signals
/// 7. Generate action report
pub async fn execute_scroll(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    target: &ScrollTarget,
    behavior: ScrollBehavior,
) -> Result<ActionReport, ActionError> {
    let started_at = Utc::now();
    let start_instant = Instant::now();

    info!(
        action_id = %ctx.action_id,
        target = ?target,
        behavior = ?behavior,
        "Executing scroll primitive"
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

    primitives.ensure_adapter_ready().await?;
    let context = primitives.resolve_context(ctx).await?;

    let mut resolved_selector = None;
    match target {
        ScrollTarget::Element(anchor) => {
            debug!("Scrolling element into view: {}", anchor.to_string());
            let resolved = primitives.resolve_anchor_selector(ctx, anchor).await?;
            scroll_element_into_view(primitives, &resolved.context, &resolved.selector, behavior)
                .await?;
            resolved_selector = Some(resolved);
        }
        _ => {
            debug!("Calculating scroll coordinates");
            let command = calculate_scroll_position(primitives, ctx, &context, target).await?;
            perform_scroll(primitives, ctx, &context, command, behavior).await?;
        }
    }

    // 4. Wait for smooth scroll to complete
    if behavior == ScrollBehavior::Smooth {
        debug!("Waiting for smooth scroll animation");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    // 5. Capture post-signals
    let post_signals = capture_post_signals(primitives, ctx).await;

    // 6. Generate report
    let latency_ms = start_instant.elapsed().as_millis() as u64;

    info!(
        action_id = %ctx.action_id,
        latency_ms = latency_ms,
        "Scroll completed successfully"
    );

    let mut report = ActionReport::success(started_at, latency_ms).with_signals(post_signals);
    if let Some(meta) = &resolved_selector {
        report = apply_resolution_metadata(report, meta);
    }
    Ok(report)
}

/// Calculate scroll position based on target
async fn calculate_scroll_position(
    primitives: &DefaultActionPrimitives,
    _ctx: &ExecCtx,
    context: &Arc<ResolvedExecutionContext>,
    target: &ScrollTarget,
) -> Result<ScrollCommand, ActionError> {
    match target {
        ScrollTarget::Top => Ok(ScrollCommand::Absolute { x: 0, y: 0 }),
        ScrollTarget::Bottom => {
            let expression = "(() => {\n                const scroller = document.scrollingElement || document.documentElement || document.body;\n                const currentX = Math.floor(window.scrollX || scroller.scrollLeft || 0);\n                const maxY = Math.max((scroller.scrollHeight || 0) - window.innerHeight, 0);\n                return { x: currentX, y: Math.floor(maxY) };\n            })()";

            let value = primitives
                .adapter()
                .evaluate_script_in_context(context, expression)
                .await
                .map_err(|err| ActionError::CdpIo(err.to_string()))?;

            let x = value.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = value.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

            Ok(ScrollCommand::Absolute { x, y })
        }
        ScrollTarget::Element(_) => Err(ActionError::ScrollTargetInvalid(
            "Element targets are handled separately".to_string(),
        )),
        ScrollTarget::Pixels(delta) => Ok(ScrollCommand::Relative { dx: 0, dy: *delta }),
    }
}

/// Perform the scroll operation
async fn perform_scroll(
    primitives: &DefaultActionPrimitives,
    _ctx: &ExecCtx,
    context: &Arc<ResolvedExecutionContext>,
    command: ScrollCommand,
    behavior: ScrollBehavior,
) -> Result<(), ActionError> {
    let behavior_str = match behavior {
        ScrollBehavior::Smooth => "smooth",
        ScrollBehavior::Instant => "auto",
    };

    let expression = match command {
        ScrollCommand::Absolute { x, y } => format!(
            "(() => {{ window.scrollTo({{ left: {x}, top: {y}, behavior: '{behavior}' }}); return true; }})()",
            x = x,
            y = y,
            behavior = behavior_str,
        ),
        ScrollCommand::Relative { dx, dy } => format!(
            "(() => {{ window.scrollBy({{ left: {dx}, top: {dy}, behavior: '{behavior}' }}); return true; }})()",
            dx = dx,
            dy = dy,
            behavior = behavior_str,
        ),
    };

    primitives
        .adapter()
        .evaluate_script_in_context(context, &expression)
        .await
        .map_err(|err| ActionError::CdpIo(err.to_string()))?;

    Ok(())
}

/// Capture post-scroll signals
async fn capture_post_signals(primitives: &DefaultActionPrimitives, ctx: &ExecCtx) -> PostSignals {
    match primitives.capture_page_signals(ctx).await {
        Ok(signals) => signals,
        Err(err) => {
            warn!("failed to capture scroll signals: {}", err);
            PostSignals::default()
        }
    }
}

enum ScrollCommand {
    Absolute { x: i32, y: i32 },
    Relative { dx: i32, dy: i32 },
}

async fn scroll_element_into_view(
    primitives: &DefaultActionPrimitives,
    context: &Arc<ResolvedExecutionContext>,
    selector: &str,
    behavior: ScrollBehavior,
) -> Result<(), ActionError> {
    let selector_literal = serde_json::to_string(selector)
        .map_err(|err| ActionError::Internal(format!("invalid selector encoding: {}", err)))?;
    let behavior_str = match behavior {
        ScrollBehavior::Smooth => "smooth",
        ScrollBehavior::Instant => "auto",
    };

    let expression = format!(
        "(() => {{\n            const el = document.querySelector({selector});\n            if (!el) {{ return {{ status: 'missing' }}; }}\n            if (typeof el.scrollIntoView === 'function') {{\n                el.scrollIntoView({{ behavior: '{behavior}', block: 'center', inline: 'nearest' }});\n            }} else {{\n                const rect = el.getBoundingClientRect();\n                window.scrollTo({{ left: rect.left + window.scrollX, top: rect.top + window.scrollY, behavior: '{behavior}' }});\n            }}\n            return {{ status: 'ok' }};\n        }})()",
        selector = selector_literal,
        behavior = behavior_str,
    );

    let value = primitives
        .adapter()
        .evaluate_script_in_context(context, &expression)
        .await
        .map_err(|err| ActionError::CdpIo(err.to_string()))?;

    match value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
    {
        "ok" => Ok(()),
        "missing" => Err(ActionError::AnchorNotFound(
            "Scroll target element not found".to_string(),
        )),
        other => Err(ActionError::Internal(format!(
            "Unexpected element scroll status: {}",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_behavior() {
        assert_eq!(ScrollBehavior::default(), ScrollBehavior::Smooth);
    }

    #[test]
    fn test_scroll_target_pixels() {
        let target = ScrollTarget::Pixels(100);
        match target {
            ScrollTarget::Pixels(delta) => assert_eq!(delta, 100),
            _ => panic!("Wrong target type"),
        }
    }
}
