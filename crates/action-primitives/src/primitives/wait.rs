//! Wait primitive - Explicit waits for various conditions

use crate::{
    errors::ActionError,
    primitives::DefaultActionPrimitives,
    types::{ActionReport, ExecCtx, PostSignals, WaitCondition},
};
use cdp_adapter::{commands::WaitGate, Cdp, ResolvedExecutionContext};
use chrono::Utc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::{sleep, timeout};
use tracing::{debug, info, warn};

/// Execute wait primitive
///
/// Waits for the specified condition to be met, with timeout.
/// Supports various conditions:
/// - Element visibility/invisibility
/// - URL/title matching
/// - Fixed duration
/// - Network idle
///
/// Steps:
/// 1. Validate condition and context
/// 2. Set up timeout based on provided duration
/// 3. Poll condition until met or timeout
/// 4. Capture post-signals
/// 5. Generate action report
pub async fn execute_wait(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    condition: &WaitCondition,
    timeout_ms: u64,
) -> Result<ActionReport, ActionError> {
    let started_at = Utc::now();
    let start_instant = Instant::now();

    info!(
        action_id = %ctx.action_id,
        condition = ?condition,
        timeout_ms = timeout_ms,
        "Executing wait primitive"
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

    // 2. Execute wait with timeout
    let timeout_duration = Duration::from_millis(timeout_ms);

    match timeout(
        timeout_duration,
        wait_for_condition(primitives, ctx, condition),
    )
    .await
    {
        Ok(Ok(())) => {
            debug!("Wait condition met successfully");
        }
        Ok(Err(e)) => {
            warn!("Wait condition check failed: {}", e);
            return Err(e);
        }
        Err(_) => {
            warn!("Wait timed out after {}ms", timeout_ms);
            return Err(ActionError::WaitTimeout(format!(
                "Condition not met after {}ms: {:?}",
                timeout_ms, condition
            )));
        }
    }

    // 3. Capture post-signals
    let post_signals = capture_post_signals(primitives, ctx).await;

    // 4. Generate report
    let latency_ms = start_instant.elapsed().as_millis() as u64;

    info!(
        action_id = %ctx.action_id,
        latency_ms = latency_ms,
        "Wait completed successfully"
    );

    Ok(ActionReport::success(started_at, latency_ms).with_signals(post_signals))
}

/// Wait for condition to be met
async fn wait_for_condition(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    condition: &WaitCondition,
) -> Result<(), ActionError> {
    match condition {
        WaitCondition::ElementVisible(anchor) => {
            wait_element_visible(primitives, ctx, anchor).await
        }
        WaitCondition::ElementHidden(anchor) => wait_element_hidden(primitives, ctx, anchor).await,
        WaitCondition::UrlMatches(pattern) => wait_url_matches(primitives, ctx, pattern).await,
        WaitCondition::UrlEquals(expected) => wait_url_equals(primitives, ctx, expected).await,
        WaitCondition::TitleMatches(pattern) => wait_title_matches(primitives, ctx, pattern).await,
        WaitCondition::Duration(ms) => wait_duration(*ms).await,
        WaitCondition::NetworkIdle(quiet_ms) => wait_network_idle(primitives, ctx, *quiet_ms).await,
    }
}

/// Wait for element to become visible
async fn wait_element_visible(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    anchor: &crate::types::AnchorDescriptor,
) -> Result<(), ActionError> {
    debug!("Waiting for element to be visible: {}", anchor.to_string());
    let resolved = primitives.resolve_anchor_selector(ctx, anchor).await?;
    wait_for_visibility(
        primitives,
        ctx,
        resolved.context.clone(),
        &resolved.selector,
        true,
    )
    .await
}

/// Wait for element to become hidden
async fn wait_element_hidden(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    anchor: &crate::types::AnchorDescriptor,
) -> Result<(), ActionError> {
    debug!("Waiting for element to be hidden: {}", anchor.to_string());
    let resolved = primitives.resolve_anchor_selector(ctx, anchor).await?;
    wait_for_visibility(
        primitives,
        ctx,
        resolved.context.clone(),
        &resolved.selector,
        false,
    )
    .await
}

/// Wait for URL to match pattern
async fn wait_url_matches(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    pattern: &str,
) -> Result<(), ActionError> {
    debug!("Waiting for URL to match: {}", pattern);
    wait_property_matches(
        primitives,
        ctx,
        pattern,
        "window.location.href || ''",
        "URL",
    )
    .await
}

/// Wait for URL to equal literal string
async fn wait_url_equals(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    expected: &str,
) -> Result<(), ActionError> {
    debug!("Waiting for URL to equal: {}", expected);
    wait_property_equals(
        primitives,
        ctx,
        expected,
        "window.location.href || ''",
        "URL",
    )
    .await
}

/// Wait for title to match pattern
async fn wait_title_matches(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    pattern: &str,
) -> Result<(), ActionError> {
    debug!("Waiting for title to match: {}", pattern);
    wait_property_matches(primitives, ctx, pattern, "document.title || ''", "title").await
}

/// Wait for fixed duration
async fn wait_duration(ms: u64) -> Result<(), ActionError> {
    debug!("Waiting for fixed duration: {}ms", ms);

    sleep(Duration::from_millis(ms)).await;
    Ok(())
}

/// Wait for network to be idle
async fn wait_network_idle(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    quiet_ms: u64,
) -> Result<(), ActionError> {
    debug!("Waiting for network idle ({}ms quiet)", quiet_ms);
    let context = primitives.resolve_context(ctx).await?;
    let window_ms = quiet_ms.max(1);
    let gate_json = serde_json::to_string(&WaitGate::NetworkQuiet {
        window_ms,
        max_inflight: 0,
    })
    .map_err(|err| ActionError::Internal(format!("failed to encode wait gate: {}", err)))?;

    primitives
        .adapter()
        .wait_basic(context.page, gate_json, ctx.remaining_time())
        .await
        .map_err(|err| ActionError::CdpIo(err.to_string()))
}

/// Capture post-wait signals
async fn capture_post_signals(primitives: &DefaultActionPrimitives, ctx: &ExecCtx) -> PostSignals {
    match primitives.capture_page_signals(ctx).await {
        Ok(signals) => signals,
        Err(err) => {
            warn!("failed to capture wait signals: {}", err);
            PostSignals::default()
        }
    }
}

async fn wait_for_visibility(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    context: Arc<ResolvedExecutionContext>,
    selector: &str,
    expect_visible: bool,
) -> Result<(), ActionError> {
    let selector_literal = serde_json::to_string(selector)
        .map_err(|err| ActionError::Internal(format!("invalid selector encoding: {}", err)))?;

    let expression = format!(
        "(() => {{\n            const el = document.querySelector({selector});\n            if (!el) {{ return {{ status: 'missing', visible: false }}; }}\n            const style = window.getComputedStyle(el);\n            const rect = el.getBoundingClientRect();\n            const visible = style.visibility !== 'hidden' && style.display !== 'none' && (rect.width > 0 || rect.height > 0 || el.getClientRects().length > 0);\n            return {{ status: 'ok', visible }};\n        }})()",
        selector = selector_literal,
    );

    loop {
        if ctx.is_cancelled() {
            return Err(ActionError::Interrupted("Context cancelled".to_string()));
        }
        if ctx.is_timeout() {
            return Err(ActionError::WaitTimeout(
                "Context deadline exceeded while waiting for visibility".to_string(),
            ));
        }

        let result = primitives
            .adapter()
            .evaluate_script_in_context(&context, &expression)
            .await
            .map_err(|err| ActionError::CdpIo(err.to_string()))?;

        match result
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
        {
            "missing" => {
                if !expect_visible {
                    return Ok(());
                }
            }
            "ok" => {
                let visible = result
                    .get("visible")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if expect_visible && visible {
                    return Ok(());
                }
                if !expect_visible && !visible {
                    return Ok(());
                }
            }
            other => {
                return Err(ActionError::Internal(format!(
                    "Unexpected visibility status: {}",
                    other
                )));
            }
        }

        sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_property_matches(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    pattern: &str,
    fetch_expr: &str,
    description: &str,
) -> Result<(), ActionError> {
    let context = primitives.resolve_context(ctx).await?;
    let pattern_literal = serde_json::to_string(pattern)
        .map_err(|err| ActionError::Internal(format!("invalid pattern encoding: {}", err)))?;

    let expression = format!(
        "(() => {{\n            const value = {fetch};\n            const pattern = {pattern};\n            let matches = false;\n            try {{\n                const regex = new RegExp(pattern);\n                matches = regex.test(value);\n            }} catch (err) {{\n                matches = value.includes(pattern);\n            }}\n            return {{ matches, value }};\n        }})()",
        fetch = fetch_expr,
        pattern = pattern_literal,
    );

    loop {
        if ctx.is_cancelled() {
            return Err(ActionError::Interrupted("Context cancelled".to_string()));
        }
        if ctx.is_timeout() {
            return Err(ActionError::WaitTimeout(format!(
                "Context deadline exceeded while waiting for {}",
                description
            )));
        }

        let result = primitives
            .adapter()
            .evaluate_script_in_context(&context, &expression)
            .await
            .map_err(|err| ActionError::CdpIo(err.to_string()))?;

        if result
            .get("matches")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(());
        }

        sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_property_equals(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    expected: &str,
    fetch_expr: &str,
    description: &str,
) -> Result<(), ActionError> {
    let context = primitives.resolve_context(ctx).await?;
    let expected_literal = serde_json::to_string(expected)
        .map_err(|err| ActionError::Internal(format!("invalid literal encoding: {}", err)))?;

    let expression = format!(
        "(() => {{\n            const value = {fetch};\n            const expected = {expected};\n            return {{ matches: value === expected, value }};\n        }})()",
        fetch = fetch_expr,
        expected = expected_literal,
    );

    loop {
        if ctx.is_cancelled() {
            return Err(ActionError::Interrupted("Context cancelled".to_string()));
        }
        if ctx.is_timeout() {
            return Err(ActionError::WaitTimeout(format!(
                "Context deadline exceeded while waiting for {}",
                description
            )));
        }

        let result = primitives
            .adapter()
            .evaluate_script_in_context(&context, &expression)
            .await
            .map_err(|err| ActionError::CdpIo(err.to_string()))?;

        if result
            .get("matches")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(());
        }

        sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_wait_duration() {
        let start = Instant::now();
        wait_duration(100).await.unwrap();
        let elapsed = start.elapsed().as_millis();

        // Should wait approximately 100ms (allow 50ms tolerance)
        assert!(elapsed >= 100);
        assert!(elapsed < 150);
    }

    #[test]
    fn test_wait_condition_variants() {
        use crate::types::{AnchorDescriptor, WaitCondition};

        let _ = WaitCondition::Duration(1000);
        let _ = WaitCondition::NetworkIdle(500);
        let _ = WaitCondition::UrlMatches("example.com".to_string());
        let _ = WaitCondition::UrlEquals("https://example.com".to_string());
        let _ = WaitCondition::TitleMatches("Home".to_string());
        let _ = WaitCondition::ElementVisible(AnchorDescriptor::Css("#button".to_string()));
        let _ = WaitCondition::ElementHidden(AnchorDescriptor::Css("#spinner".to_string()));
    }
}
