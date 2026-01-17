//! Navigate primitive - Navigate to URL with built-in waiting

use crate::{
    errors::ActionError,
    primitives::DefaultActionPrimitives,
    types::{ActionReport, ExecCtx, PostSignals, WaitTier},
};
use cdp_adapter::Cdp;
use chrono::Utc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Execute navigate primitive
///
/// Navigates to the specified URL with built-in waiting based on tier.
/// Default tier: Idle (wait for DOM ready + network quiet)
///
/// Steps:
/// 1. Validate URL format
/// 2. Check execution context (not cancelled, not timeout)
/// 3. Issue CDP navigate command
/// 4. Apply built-in waiting based on tier
/// 5. Capture post-signals (URL, title)
/// 6. Generate action report
pub async fn execute_navigate(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    url: &str,
    wait_tier: WaitTier,
) -> Result<ActionReport, ActionError> {
    let started_at = Utc::now();
    let start_instant = Instant::now();

    info!(
        action_id = %ctx.action_id,
        url = %url,
        wait_tier = ?wait_tier,
        "Executing navigate primitive"
    );

    // 1. Validate URL
    if url.is_empty() {
        return Err(ActionError::Internal("URL cannot be empty".to_string()));
    }

    // Basic URL validation
    if !url.starts_with("http://") && !url.starts_with("https://") && !url.starts_with("file://") {
        return Err(ActionError::Internal(format!(
            "Invalid URL scheme: {}",
            url
        )));
    }

    // 2. Check context
    if ctx.is_cancelled() {
        return Err(ActionError::Interrupted("Context cancelled".to_string()));
    }

    if ctx.is_timeout() {
        return Err(ActionError::NavTimeout(
            "Context deadline exceeded".to_string(),
        ));
    }

    // 3. Execute CDP navigate
    debug!("Issuing CDP Page.navigate command");
    primitives.ensure_adapter_ready().await?;
    let context = primitives.resolve_context(ctx).await?;
    let page_id = context.page;
    primitives
        .adapter()
        .navigate(page_id, url, ctx.remaining_time())
        .await
        .map_err(|err| ActionError::CdpIo(err.to_string()))?;

    // 4. Apply built-in waiting
    if wait_tier != WaitTier::None {
        debug!("Applying built-in wait tier: {:?}", wait_tier);
        primitives
            .wait_strategy()
            .wait(primitives.adapter().clone(), page_id, wait_tier)
            .await?;
    }

    // 5. Capture post-signals
    let post_signals = capture_post_signals(primitives, ctx).await;

    // 6. Generate report
    let latency_ms = start_instant.elapsed().as_millis() as u64;

    info!(
        action_id = %ctx.action_id,
        latency_ms = latency_ms,
        url_after = ?post_signals.url_after,
        "Navigate completed successfully"
    );

    Ok(ActionReport::success(started_at, latency_ms).with_signals(post_signals))
}

/// Capture post-navigation signals
async fn capture_post_signals(primitives: &DefaultActionPrimitives, ctx: &ExecCtx) -> PostSignals {
    match primitives.capture_page_signals(ctx).await {
        Ok(signals) => signals,
        Err(err) => {
            warn!("failed to capture navigation signals: {}", err);
            PostSignals::default()
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_url_validation() {
        // Valid URLs
        assert!(validate_url("https://example.com"));
        assert!(validate_url("http://localhost:8080"));
        assert!(validate_url("file:///path/to/file.html"));

        // Invalid URLs
        assert!(!validate_url(""));
        assert!(!validate_url("example.com"));
        assert!(!validate_url("ftp://example.com"));
    }

    fn validate_url(url: &str) -> bool {
        !url.is_empty()
            && (url.starts_with("http://")
                || url.starts_with("https://")
                || url.starts_with("file://"))
    }
}
