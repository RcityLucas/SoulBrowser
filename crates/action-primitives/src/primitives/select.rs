//! Select primitive - Select from dropdown/listbox

use crate::{
    errors::ActionError,
    locator::apply_resolution_metadata,
    primitives::DefaultActionPrimitives,
    types::{ActionReport, AnchorDescriptor, ExecCtx, PostSignals, SelectMethod, WaitTier},
};
use cdp_adapter::{commands::SelectSpec, AdapterErrorKind, Cdp, ResolvedExecutionContext};
use chrono::Utc;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Execute select primitive
///
/// Selects an option from dropdown or listbox.
/// Supports selection by text, value, or index.
///
/// Steps:
/// 1. Validate anchor, method, item, and context
/// 2. Resolve select element via locator
/// 3. Check element is selectable (select/listbox, enabled, not readonly)
/// 4. Find matching option
/// 5. Select option via CDP
/// 6. Apply built-in waiting based on tier
/// 7. Capture post-signals
/// 8. Generate action report
pub async fn execute_select(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    anchor: &AnchorDescriptor,
    method: SelectMethod,
    item: &str,
    wait_tier: WaitTier,
) -> Result<ActionReport, ActionError> {
    let started_at = Utc::now();
    let start_instant = Instant::now();

    info!(
        action_id = %ctx.action_id,
        anchor = %anchor.to_string(),
        method = ?method,
        item = %item,
        wait_tier = ?wait_tier,
        "Executing select primitive"
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

    // 2. Resolve select element
    debug!(
        "Resolving select element via anchor: {}",
        anchor.to_string()
    );
    let resolved = primitives.resolve_anchor_selector(ctx, anchor).await?;
    let selector = resolved.selector.clone();
    let context = resolved.context.clone();
    let page_id = context.page;

    // 3. Check selectability
    debug!("Checking element is selectable");
    check_selectable(primitives, &context, &selector).await?;

    // 4. Find matching option
    debug!("Finding option by {:?}: {}", method, item);
    let (option_value, match_label) =
        find_option(primitives, ctx, &context, &selector, method, item).await?;

    // 5. Select option
    debug!("Selecting option: {}", option_value);
    perform_select(
        primitives,
        ctx,
        &context,
        &selector,
        &option_value,
        match_label,
    )
    .await?;

    // 6. Apply built-in waiting
    if wait_tier != WaitTier::None {
        debug!("Applying built-in wait tier: {:?}", wait_tier);
        primitives
            .wait_strategy()
            .wait(primitives.adapter().clone(), page_id, wait_tier)
            .await?;
    }

    // 7. Capture post-signals
    let post_signals = capture_post_signals(primitives, ctx).await;

    // 8. Generate report
    let latency_ms = start_instant.elapsed().as_millis() as u64;

    info!(
        action_id = %ctx.action_id,
        latency_ms = latency_ms,
        "Select completed successfully"
    );

    let report = ActionReport::success(started_at, latency_ms).with_signals(post_signals);
    Ok(apply_resolution_metadata(report, &resolved))
}

/// Check if element is selectable
async fn check_selectable(
    primitives: &DefaultActionPrimitives,
    context: &Arc<ResolvedExecutionContext>,
    selector: &str,
) -> Result<(), ActionError> {
    let selector_literal = serde_json::to_string(selector)
        .map_err(|err| ActionError::Internal(format!("invalid selector encoding: {}", err)))?;

    let expression = format!(
        "(() => {{\n            const el = document.querySelector({selector});\n            if (!el) {{ return {{ status: 'missing' }}; }}\n            const tag = (el.tagName || '').toLowerCase();\n            const role = (el.getAttribute('role') || '').toLowerCase();\n            const selectable = tag === 'select' || role === 'listbox';\n            const disabled = !!el.matches(':disabled');\n            const readonly = el.hasAttribute('readonly');\n            const style = window.getComputedStyle(el);\n            const rect = el.getBoundingClientRect();\n            const visible = selectable && style.visibility !== 'hidden' && style.display !== 'none' && (rect.width > 0 || rect.height > 0 || el.getClientRects().length > 0);\n            return {{ status: 'ok', selectable, disabled, readonly, visible }};\n        }})()",
        selector = selector_literal
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
        "missing" => Err(ActionError::AnchorNotFound(
            "Select element not found".to_string(),
        )),
        "ok" => {
            if !value
                .get("selectable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return Err(ActionError::Internal(
                    "Target element is not a select/listbox".to_string(),
                ));
            }

            if value
                .get("disabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return Err(ActionError::NotEnabled(
                    "Select element is disabled".to_string(),
                ));
            }

            if value
                .get("readonly")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return Err(ActionError::NotEnabled(
                    "Select element is readonly".to_string(),
                ));
            }

            if !value
                .get("visible")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return Err(ActionError::NotClickable(
                    "Select element is not visible".to_string(),
                ));
            }

            Ok(())
        }
        other => Err(ActionError::Internal(format!(
            "Unexpected selectability status: {}",
            other
        ))),
    }
}

/// Find option matching the selection criteria
async fn find_option(
    primitives: &DefaultActionPrimitives,
    _ctx: &ExecCtx,
    context: &Arc<ResolvedExecutionContext>,
    selector: &str,
    method: SelectMethod,
    item: &str,
) -> Result<(String, bool), ActionError> {
    match method {
        SelectMethod::Text => {
            ensure_option_exists(primitives, context, selector, "text", item).await?;
            Ok((item.to_string(), true))
        }
        SelectMethod::Value => {
            ensure_option_exists(primitives, context, selector, "value", item).await?;
            Ok((item.to_string(), false))
        }
        SelectMethod::Index => {
            let idx = item
                .parse::<usize>()
                .map_err(|_| ActionError::OptionNotFound(format!("Invalid index: {}", item)))?;
            let value = option_value_by_index(primitives, context, selector, idx).await?;
            Ok((value, false))
        }
    }
}

async fn ensure_option_exists(
    primitives: &DefaultActionPrimitives,
    context: &Arc<ResolvedExecutionContext>,
    selector: &str,
    mode: &str,
    needle: &str,
) -> Result<(), ActionError> {
    let selector_literal = serde_json::to_string(selector)
        .map_err(|err| ActionError::Internal(format!("invalid selector encoding: {}", err)))?;
    let needle_literal = serde_json::to_string(needle)
        .map_err(|err| ActionError::Internal(format!("invalid option literal: {}", err)))?;

    let comparator = if mode == "value" {
        "(opt.value ?? '') === target"
    } else {
        "(opt.text ?? '') === target"
    };

    let expression = format!(
        "(() => {{\n            const root = document.querySelector({selector});\n            if (!root) {{ return {{ status: 'missing' }}; }}\n            const options = Array.from(root.options || []);\n            const target = {needle};\n            const matchOpt = options.find(opt => {comparator});\n            if (!matchOpt) {{ return {{ status: 'not-found' }}; }}\n            return {{ status: 'ok' }};\n        }})()",
        selector = selector_literal,
        needle = needle_literal,
        comparator = comparator,
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
            "Select element not found".to_string(),
        )),
        "not-found" => Err(ActionError::OptionNotFound(match mode {
            "value" => format!("Option with value '{}' not found", needle),
            "text" => format!("Option with text '{}' not found", needle),
            _ => "Option not found".to_string(),
        })),
        other => Err(ActionError::Internal(format!(
            "Unexpected option lookup status: {}",
            other
        ))),
    }
}

async fn option_value_by_index(
    primitives: &DefaultActionPrimitives,
    context: &Arc<ResolvedExecutionContext>,
    selector: &str,
    index: usize,
) -> Result<String, ActionError> {
    let selector_literal = serde_json::to_string(selector)
        .map_err(|err| ActionError::Internal(format!("invalid selector encoding: {}", err)))?;

    let expression = format!(
        "(() => {{\n            const root = document.querySelector({selector});\n            if (!root) {{ return {{ status: 'missing' }}; }}\n            const options = Array.from(root.options || []);\n            const idx = {index};\n            if (idx < 0 || idx >= options.length) {{\n                return {{ status: 'out-of-range', length: options.length }};\n            }}\n            const opt = options[idx];\n            const value = typeof opt.value === 'string' ? opt.value : '';\n            return {{ status: 'ok', value }};\n        }})()",
        selector = selector_literal,
        index = index,
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
        "ok" => value
            .get("value")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| ActionError::Internal("Missing option value".to_string())),
        "missing" => Err(ActionError::AnchorNotFound(
            "Select element not found".to_string(),
        )),
        "out-of-range" => Err(ActionError::OptionNotFound(format!(
            "Option index {} out of range",
            index
        ))),
        other => Err(ActionError::Internal(format!(
            "Unexpected index lookup status: {}",
            other
        ))),
    }
}

/// Perform the selection
async fn perform_select(
    primitives: &DefaultActionPrimitives,
    ctx: &ExecCtx,
    context: &Arc<ResolvedExecutionContext>,
    selector: &str,
    option_value: &str,
    match_label: bool,
) -> Result<(), ActionError> {
    let spec = SelectSpec {
        selector: selector.to_string(),
        value: option_value.to_string(),
        match_label,
    };

    primitives
        .adapter()
        .select_option(context.page, spec, ctx.remaining_time())
        .await
        .map_err(|err| {
            let hint = err.hint.clone();
            let message = err.to_string();
            match err.kind {
                AdapterErrorKind::TargetNotFound => {
                    let detail = hint.unwrap_or_else(|| {
                        format!("Select element not found for selector '{}'", selector)
                    });
                    ActionError::AnchorNotFound(detail)
                }
                AdapterErrorKind::OptionNotFound => {
                    let detail = hint.unwrap_or_else(|| "Select option not found".to_string());
                    ActionError::OptionNotFound(detail)
                }
                AdapterErrorKind::NavTimeout => {
                    let detail = hint.unwrap_or(message.clone());
                    ActionError::WaitTimeout(detail)
                }
                AdapterErrorKind::PolicyDenied => ActionError::PolicyDenied(message),
                AdapterErrorKind::CdpIo | AdapterErrorKind::Internal => ActionError::CdpIo(message),
            }
        })
}

/// Capture post-selection signals
async fn capture_post_signals(primitives: &DefaultActionPrimitives, ctx: &ExecCtx) -> PostSignals {
    match primitives.capture_page_signals(ctx).await {
        Ok(signals) => signals,
        Err(err) => {
            warn!("failed to capture select signals: {}", err);
            PostSignals::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_method() {
        assert_eq!(SelectMethod::Text as u8, 0);
        assert_eq!(SelectMethod::Value as u8, 1);
        assert_eq!(SelectMethod::Index as u8, 2);
    }

    #[test]
    fn test_index_parsing() {
        assert!(parse_index("0").is_ok());
        assert!(parse_index("5").is_ok());
        assert!(parse_index("invalid").is_err());
        assert!(parse_index("-1").is_err());
    }

    fn parse_index(s: &str) -> Result<usize, std::num::ParseIntError> {
        s.parse::<usize>()
    }
}
