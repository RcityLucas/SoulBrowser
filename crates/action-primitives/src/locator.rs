use crate::{
    errors::ActionError,
    primitives::DefaultActionPrimitives,
    types::{ActionReport, AnchorDescriptor, ExecCtx, SelfHealInfo},
};
use async_trait::async_trait;
use cdp_adapter::{Cdp, ResolvedExecutionContext};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

/// Resolver responsible for turning an `AnchorDescriptor` into a concrete selector
/// that CDP commands can operate on. Different implementations can plug-in
/// multi-strategy locators or self-healing logic.
#[async_trait]
pub trait AnchorResolver: Send + Sync {
    async fn resolve(
        &self,
        primitives: &DefaultActionPrimitives,
        ctx: &ExecCtx,
        anchor: &AnchorDescriptor,
    ) -> Result<ResolvedSelector, ActionError>;
}

/// Resolution outcome returned by an [`AnchorResolver`].
#[derive(Clone, Debug)]
pub struct ResolvedSelector {
    pub selector: String,
    pub context: Arc<ResolvedExecutionContext>,
    pub strategy: Option<String>,
    pub confidence: Option<f64>,
    pub heal_info: Option<SelfHealInfo>,
}

impl ResolvedSelector {
    pub fn new(selector: String, context: Arc<ResolvedExecutionContext>) -> Self {
        Self {
            selector,
            context,
            strategy: None,
            confidence: None,
            heal_info: None,
        }
    }
}

/// Default script-based resolver that directly evaluates anchors in the page.
#[derive(Default)]
pub struct ScriptAnchorResolver;

#[async_trait]
impl AnchorResolver for ScriptAnchorResolver {
    async fn resolve(
        &self,
        primitives: &DefaultActionPrimitives,
        ctx: &ExecCtx,
        anchor: &AnchorDescriptor,
    ) -> Result<ResolvedSelector, ActionError> {
        primitives.ensure_adapter_ready().await?;
        let context = primitives.resolve_route_context(&ctx.route).await?;
        let selector = resolve_anchor_on_page(primitives, &context, anchor).await?;
        Ok(ResolvedSelector::new(selector, context))
    }
}

/// Attach resolution metadata (e.g., self-heal info) to an [`ActionReport`].
pub fn apply_resolution_metadata(
    mut report: ActionReport,
    resolved: &ResolvedSelector,
) -> ActionReport {
    if let Some(heal) = &resolved.heal_info {
        report = report.with_heal(heal.clone());
    }
    report
}

async fn resolve_anchor_on_page(
    primitives: &DefaultActionPrimitives,
    context: &Arc<ResolvedExecutionContext>,
    anchor: &AnchorDescriptor,
) -> Result<String, ActionError> {
    match anchor {
        AnchorDescriptor::Css(selector) => {
            let trimmed = selector.trim();
            if trimmed.is_empty() {
                return Err(ActionError::AnchorNotFound(
                    "Empty CSS selector".to_string(),
                ));
            }
            Ok(trimmed.to_string())
        }
        AnchorDescriptor::Aria { role, name } => {
            if role.trim().is_empty() || name.trim().is_empty() {
                return Err(ActionError::AnchorNotFound(
                    "ARIA role and name must be provided".to_string(),
                ));
            }
            resolve_aria_selector(primitives, context, role, name).await
        }
        AnchorDescriptor::Text { content, exact } => {
            if content.trim().is_empty() {
                return Err(ActionError::AnchorNotFound(
                    "Text content cannot be empty".to_string(),
                ));
            }
            resolve_text_selector(primitives, context, content, *exact).await
        }
    }
}

async fn resolve_aria_selector(
    primitives: &DefaultActionPrimitives,
    context: &Arc<ResolvedExecutionContext>,
    role: &str,
    name: &str,
) -> Result<String, ActionError> {
    let token = format!("aria-{}", Uuid::new_v4().simple());
    let expression = format!(
        r#"(() => {{
            const role = {role};
            const targetName = {name};
            const attr = {attr};
            const token = {token};
            const normalize = (input) => (input || '').trim().toLowerCase();
            const computeName = (el) => {{
                if (!el) return '';
                const label = el.getAttribute('aria-label');
                if (label) return label.trim();
                const labelledby = el.getAttribute('aria-labelledby');
                if (labelledby) {{
                    return labelledby.split(/\s+/)
                        .map(id => document.getElementById(id))
                        .map(node => node ? (node.textContent || '') : '')
                        .join(' ')
                        .trim();
                }}
                if (el.title) return el.title.trim();
                return (el.innerText || el.textContent || '').trim();
            }};
            const matches = Array.from(document.querySelectorAll('[role="' + role + '"]'));
            const match = matches.find(el => normalize(computeName(el)) === normalize(targetName));
            if (!match) {{
                return {{ status: 'not-found' }};
            }}
            match.setAttribute(attr, token);
            return {{ status: 'ok', selector: '[' + attr + '="' + token + '"]' }};
        }})()"#,
        role = serde_json::to_string(role).unwrap(),
        name = serde_json::to_string(name).unwrap(),
        attr = serde_json::to_string("data-soulbrowser-anchor").unwrap(),
        token = serde_json::to_string(&token).unwrap(),
    );

    evaluate_selector_script(primitives, context, &expression, "ARIA descriptor").await
}

async fn resolve_text_selector(
    primitives: &DefaultActionPrimitives,
    context: &Arc<ResolvedExecutionContext>,
    text: &str,
    exact: bool,
) -> Result<String, ActionError> {
    let token = format!("text-{}", Uuid::new_v4().simple());
    let expression = format!(
        r#"(() => {{
            const target = {text};
            const attr = {attr};
            const token = {token};
            const exact = {exact};
            const normalize = (input) => (input || '').trim();
            const lower = (input) => normalize(input).toLowerCase();
            const isVisible = (el) => {{
                if (!(el instanceof Element)) return false;
                const style = window.getComputedStyle(el);
                if (style.visibility === 'hidden' || style.display === 'none') return false;
                const rect = el.getBoundingClientRect();
                return rect.width > 0 || rect.height > 0 || el.getClientRects().length > 0;
            }};
            const nodes = Array.from(document.querySelectorAll('body *'));
            const match = nodes.find(el => {{
                if (!isVisible(el)) return false;
                const value = normalize(el.innerText || el.textContent || '');
                if (!value) return false;
                if (exact) {{
                    return lower(value) === lower(target);
                }}
                return lower(value).includes(lower(target));
            }});
            if (!match) {{
                return {{ status: 'not-found' }};
            }}
            match.setAttribute(attr, token);
            return {{ status: 'ok', selector: '[' + attr + '="' + token + '"]' }};
        }})()"#,
        text = serde_json::to_string(text).unwrap(),
        attr = serde_json::to_string("data-soulbrowser-anchor").unwrap(),
        token = serde_json::to_string(&token).unwrap(),
        exact = if exact { "true" } else { "false" },
    );

    evaluate_selector_script(primitives, context, &expression, "text anchor").await
}

async fn evaluate_selector_script(
    primitives: &DefaultActionPrimitives,
    context: &Arc<ResolvedExecutionContext>,
    expression: &str,
    label: &str,
) -> Result<String, ActionError> {
    let value: Value = primitives
        .adapter()
        .evaluate_script_in_context(context, expression)
        .await
        .map_err(|err| ActionError::CdpIo(err.to_string()))?;

    extract_selector(value).ok_or_else(|| {
        ActionError::AnchorNotFound(format!("{} did not resolve to a visible element", label))
    })
}

fn extract_selector(value: Value) -> Option<String> {
    let status = value.get("status").and_then(|v| v.as_str())?;
    match status {
        "ok" => value
            .get("selector")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        primitives::DefaultActionPrimitives,
        types::{AnchorDescriptor, ExecCtx},
        waiting::DefaultWaitStrategy,
    };
    use cdp_adapter::{config::CdpConfig, event_bus, transport::CdpTransport, CdpAdapter};
    use soulbrowser_core_types::{ExecRoute, FrameId, PageId as RoutePageId, SessionId};
    use soulbrowser_policy_center::{default_snapshot, PolicyView};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    #[derive(Default)]
    struct ResolverTestTransport {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl CdpTransport for ResolverTestTransport {
        async fn start(&self) -> Result<(), cdp_adapter::error::AdapterError> {
            Ok(())
        }

        async fn next_event(&self) -> Option<cdp_adapter::transport::TransportEvent> {
            None
        }

        async fn send_command(
            &self,
            _target: cdp_adapter::transport::CommandTarget,
            method: &str,
            params: serde_json::Value,
        ) -> Result<serde_json::Value, cdp_adapter::error::AdapterError> {
            if method == "Runtime.evaluate" {
                let expression = params
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                self.calls.lock().await.push(expression.clone());
                let selector = extract_attr_and_token(&expression)
                    .unwrap_or_else(|| "[data-soulbrowser-anchor=\"mock\"]".to_string());
                return Ok(serde_json::json!({
                    "result": {
                        "value": {
                            "status": "ok",
                            "selector": selector,
                        }
                    }
                }));
            }

            if method == "Target.createTarget" {
                return Ok(serde_json::json!({ "targetId": "test-target" }));
            }

            Ok(serde_json::json!({}))
        }
    }

    fn extract_attr_and_token(expression: &str) -> Option<String> {
        fn extract_value(expr: &str, needle: &str) -> Option<String> {
            let start = expr.find(needle)? + needle.len();
            let rest = &expr[start..];
            let quote_start = rest.find('"')? + 1;
            let rest = &rest[quote_start..];
            let quote_end = rest.find('"')?;
            Some(rest[..quote_end].to_string())
        }

        let attr = extract_value(expression, "const attr = ")?;
        let token = extract_value(expression, "const token = ")?;
        Some(format!("[{attr}=\"{token}\"]"))
    }

    fn test_primitives() -> (Arc<DefaultActionPrimitives>, ExecCtx, cdp_adapter::PageId) {
        let (bus, _rx) = event_bus(8);
        let transport = Arc::new(ResolverTestTransport::default());
        let adapter = Arc::new(CdpAdapter::with_transport(
            CdpConfig::default(),
            bus,
            transport,
        ));
        let primitives = Arc::new(DefaultActionPrimitives::new(
            adapter.clone(),
            Arc::new(DefaultWaitStrategy::default()),
        ));

        let page_uuid = uuid::Uuid::new_v4();
        let session_uuid = uuid::Uuid::new_v4();
        adapter.register_page(
            cdp_adapter::PageId(page_uuid),
            cdp_adapter::SessionId(session_uuid),
            Some("target".to_string()),
            Some("session".to_string()),
        );

        let route = ExecRoute::new(
            SessionId(session_uuid.to_string()),
            RoutePageId(page_uuid.to_string()),
            FrameId(uuid::Uuid::new_v4().to_string()),
        );

        let ctx = ExecCtx::new(
            route,
            Instant::now() + Duration::from_secs(5),
            CancellationToken::new(),
            PolicyView::from(default_snapshot()),
        );

        (primitives, ctx, cdp_adapter::PageId(page_uuid))
    }

    #[tokio::test]
    async fn resolves_css_anchor_without_scripts() {
        let (primitives, ctx, page_id) = test_primitives();
        let anchor = AnchorDescriptor::Css("#login".to_string());
        let resolved = primitives
            .resolve_anchor_selector(&ctx, &anchor)
            .await
            .unwrap();
        assert_eq!(resolved.selector, "#login");
        assert_eq!(resolved.context.page, page_id);
    }

    #[tokio::test]
    async fn resolves_aria_anchor_via_script() {
        let (primitives, ctx, _) = test_primitives();
        let anchor = AnchorDescriptor::Aria {
            role: "button".to_string(),
            name: "Submit".to_string(),
        };
        let resolved = primitives
            .resolve_anchor_selector(&ctx, &anchor)
            .await
            .unwrap();
        assert!(resolved.selector.starts_with("[data-soulbrowser-anchor"));
    }

    #[tokio::test]
    async fn resolves_text_anchor_via_script() {
        let (primitives, ctx, _) = test_primitives();
        let anchor = AnchorDescriptor::Text {
            content: "Continue".to_string(),
            exact: false,
        };
        let resolved = primitives
            .resolve_anchor_selector(&ctx, &anchor)
            .await
            .unwrap();
        assert!(resolved.selector.contains("data-soulbrowser-anchor"));
    }

    #[tokio::test]
    async fn resolves_distinct_contexts_for_frames() {
        let (primitives, ctx, page_id) = test_primitives();
        let anchor = AnchorDescriptor::Css("#login".to_string());
        let resolved_main = primitives
            .resolve_anchor_selector(&ctx, &anchor)
            .await
            .unwrap();
        assert_eq!(resolved_main.context.page, page_id);
        assert_eq!(
            resolved_main.context.frame_selector.as_deref(),
            Some(ctx.route.frame.0.as_str())
        );

        let mut alt_route = ctx.route.clone();
        alt_route.frame = FrameId("[data-frame='alt']".to_string());
        let alt_ctx = ExecCtx {
            route: alt_route,
            ..ctx.clone()
        };

        let resolved_alt = primitives
            .resolve_anchor_selector(&alt_ctx, &anchor)
            .await
            .unwrap();

        assert_eq!(resolved_alt.context.page, page_id);
        assert_eq!(
            resolved_alt.context.frame_selector.as_deref(),
            Some("[data-frame='alt']")
        );
        assert!(!Arc::ptr_eq(&resolved_main.context, &resolved_alt.context));
        assert_eq!(resolved_main.selector, "#login");
        assert_eq!(resolved_alt.selector, "#login");
    }
}
