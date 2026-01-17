//! Type text primitive - Type text into input fields

use crate::{
    errors::ActionError,
    locator::apply_resolution_metadata,
    primitives::DefaultActionPrimitives,
    types::{ActionReport, AnchorDescriptor, ExecCtx, PostSignals, WaitTier},
};
use cdp_adapter::{AdapterErrorKind, Cdp, ResolvedExecutionContext};
use chrono::Utc;
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;
use tokio::time::{sleep, Duration};
use tracing::{debug, info, warn};

/// Execute type_text primitive
///
/// Types text into the element identified by anchor.
/// Optionally submits the form after typing (press Enter).
/// Optional wait_tier for post-submit waiting.
///
/// Steps:
/// 1. Validate anchor, text, and context
/// 2. Resolve element via locator
/// 3. Check element is typeable (input/textarea, enabled, not readonly)
/// 4. Focus element
/// 5. Clear existing content (optional)
/// 6. Type text character by character
/// 7. Submit if requested (press Enter)
/// 8. Apply built-in waiting if tier specified
/// 9. Capture post-signals
/// 10. Generate action report
pub async fn execute_type_text(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    anchor: &AnchorDescriptor,
    text: &str,
    submit: bool,
    wait_tier: Option<WaitTier>,
) -> Result<ActionReport, ActionError> {
    let started_at = Utc::now();
    let start_instant = Instant::now();

    info!(
        action_id = %ctx.action_id,
        anchor = %anchor.to_string(),
        text_length = text.len(),
        submit = submit,
        wait_tier = ?wait_tier,
        "Executing type_text primitive"
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

    if text.is_empty() {
        return Err(ActionError::Internal("Text cannot be empty".to_string()));
    }

    // 2. Resolve selector and page
    debug!("Resolving element via anchor: {}", anchor.to_string());
    maybe_dismiss_google_consent(primitives, ctx).await?;
    let resolved = primitives.resolve_anchor_selector(ctx, anchor).await?;
    let selector = resolved.selector.clone();
    let context = resolved.context.clone();

    // 3. Type text via CDP
    debug!("Typing {} characters", text.len());
    primitives
        .adapter()
        .type_text_in_context(&context, &selector, text, ctx.remaining_time())
        .await
        .map_err(|err| {
            let hint = err.hint.clone();
            let message = err.to_string();
            match err.kind {
                AdapterErrorKind::TargetNotFound => {
                    let detail = hint.clone().unwrap_or_else(|| {
                        format!("Selector '{}' not found before deadline", selector)
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

    // 4. Submit if requested
    if submit {
        debug!("Submitting form (pressing Enter)");
        trigger_submit(primitives, &context).await?;
    }

    // 5. Apply waiting if requested
    if let Some(tier) = wait_tier {
        debug!("Applying wait tier: {:?}", tier);
        primitives
            .wait_strategy()
            .wait(primitives.adapter().clone(), context.page, tier)
            .await?;
    }

    // 6. Capture post-signals
    let post_signals = capture_post_signals(primitives, ctx).await;

    // 7. Generate report
    let latency_ms = start_instant.elapsed().as_millis() as u64;

    info!(
        action_id = %ctx.action_id,
        latency_ms = latency_ms,
        "Type text completed successfully"
    );

    let report = ActionReport::success(started_at, latency_ms).with_signals(post_signals);
    Ok(apply_resolution_metadata(report, &resolved))
}

/// Capture post-typing signals
async fn capture_post_signals(primitives: &DefaultActionPrimitives, ctx: &ExecCtx) -> PostSignals {
    match primitives.capture_page_signals(ctx).await {
        Ok(signals) => signals,
        Err(err) => {
            warn!("failed to capture typing signals: {}", err);
            PostSignals::default()
        }
    }
}

async fn maybe_dismiss_google_consent(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
) -> Result<(), ActionError> {
    let context = match primitives.resolve_context(ctx).await {
        Ok(ctx) => ctx,
        Err(err) => return Err(err),
    };

    let script = r#"(() => {
        const hostname = (window.location && window.location.hostname) || '';
        if (!hostname.includes('google.')) {
            return { checked: false, reason: 'host-mismatch' };
        }

        const body = document.body;
        if (!body) {
            return { checked: true, handled: false, reason: 'no-body' };
        }
        const text = (body.innerText || '').toLowerCase();
        const hasConsentGate =
            text.includes('before you continue') ||
            text.includes('we use cookies') ||
            !!document.querySelector('form[action*="consent"]');
        if (!hasConsentGate) {
            return { checked: true, handled: false, reason: 'no-gate' };
        }

        const preferSelectors = ['button#L2AGLb', 'button[jsname="higCR"]'];
        for (const selector of preferSelectors) {
            const el = document.querySelector(selector);
            if (el) {
                el.click();
                return { checked: true, handled: true, strategy: 'selector', selector };
            }
        }

        const buttonCandidates = Array.from(
            document.querySelectorAll('button, input[type="button"], input[type="submit"]')
        );
        const matchers = [/accept all/i, /accept/i, /agree/i, /全部接受/, /同意/, /接受全部/];
        for (const el of buttonCandidates) {
            const label = (el.innerText || el.value || '').trim();
            if (!label) {
                continue;
            }
            if (matchers.some((regex) => regex.test(label))) {
                el.click();
                return { checked: true, handled: true, strategy: 'label', label };
            }
        }

        return { checked: true, handled: false, reason: 'button-missing' };
    })()"#;

    let value = primitives
        .adapter()
        .evaluate_script_in_context(&context, script)
        .await
        .map_err(|err| ActionError::CdpIo(err.to_string()))?;

    let handled = value
        .get("handled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let checked = value
        .get("checked")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if handled {
        let strategy = value
            .get("strategy")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        info!(
            action_id = %ctx.action_id,
            strategy = %strategy,
            "Dismissed Google consent dialog"
        );
        sleep(Duration::from_millis(250)).await;
    } else if checked {
        let reason = value
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("not-handled");
        debug!(
            action_id = %ctx.action_id,
            reason = %reason,
            "No Google consent dismissal needed"
        );
    }

    Ok(())
}

async fn trigger_submit(
    primitives: &DefaultActionPrimitives,
    context: &Arc<ResolvedExecutionContext>,
) -> Result<(), ActionError> {
    let expression = "(() => {\n    try {\n        const el = document.activeElement;\n        if (!el) { return { status: 'no-active' }; }\n        if (el.form && typeof el.form.requestSubmit === 'function') {\n            el.form.requestSubmit();\n            return { status: 'requestSubmit' };\n        }\n        if (el.form && typeof el.form.submit === 'function') {\n            el.form.submit();\n            return { status: 'submit' };\n        }\n        const keyDown = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true });\n        const keyUp = new KeyboardEvent('keyup', { key: 'Enter', bubbles: true });\n        el.dispatchEvent(keyDown);\n        el.dispatchEvent(keyUp);\n        return { status: 'key' };\n    } catch (err) {\n        return { status: 'error', message: String(err) };\n    }\n})()";

    let value = primitives
        .adapter()
        .evaluate_script_in_context(context, expression)
        .await
        .map_err(|err| ActionError::CdpIo(err.to_string()))?;

    if let Some(status) = value.get("status").and_then(|v| v.as_str()) {
        if status == "error" {
            let message = value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(ActionError::Internal(format!(
                "Failed to submit form: {}",
                message
            )));
        }
    }

    Ok(())
}
