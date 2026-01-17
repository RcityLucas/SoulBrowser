//! Action primitives implementation
//!
//! Six core primitives for browser automation:
//! 1. navigate - Navigate to URL with built-in waiting
//! 2. click - Click element with fallback strategies
//! 3. type_text - Type text into input fields
//! 4. select - Select from dropdown/listbox
//! 5. scroll - Scroll page or element into view
//! 6. wait - Explicit waits for various conditions

mod click;
mod navigate;
mod scroll;
mod select;
mod type_text;
mod wait;

pub use click::*;
pub use navigate::*;
pub use scroll::*;
pub use select::*;
pub use type_text::*;
pub use wait::*;

use async_trait::async_trait;
use cdp_adapter::{Cdp, CdpAdapter, ResolvedExecutionContext};
use dashmap::DashMap;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::OnceCell;

use crate::{
    errors::ActionError,
    locator::{AnchorResolver, ResolvedSelector, ScriptAnchorResolver},
    types::{ActionReport, AnchorDescriptor, ExecCtx, PostSignals},
    waiting::WaitStrategy,
};

/// Action primitives trait
///
/// Defines the interface for all action primitives.
/// Each primitive is responsible for:
/// - Validating execution context
/// - Executing the action via CDP
/// - Applying built-in waiting if configured
/// - Capturing signals and generating reports
#[async_trait]
pub trait ActionPrimitives: Send + Sync {
    /// Navigate to a URL
    async fn navigate(
        &self,
        ctx: &ExecCtx,
        url: &str,
        wait_tier: crate::types::WaitTier,
    ) -> Result<ActionReport, ActionError>;

    /// Click an element
    async fn click(
        &self,
        ctx: &ExecCtx,
        anchor: &crate::types::AnchorDescriptor,
        wait_tier: crate::types::WaitTier,
    ) -> Result<ActionReport, ActionError>;

    /// Type text into an element
    async fn type_text(
        &self,
        ctx: &ExecCtx,
        anchor: &crate::types::AnchorDescriptor,
        text: &str,
        submit: bool,
        wait_tier: Option<crate::types::WaitTier>,
    ) -> Result<ActionReport, ActionError>;

    /// Select from dropdown/listbox
    async fn select(
        &self,
        ctx: &ExecCtx,
        anchor: &crate::types::AnchorDescriptor,
        method: crate::types::SelectMethod,
        item: &str,
        wait_tier: crate::types::WaitTier,
    ) -> Result<ActionReport, ActionError>;

    /// Scroll to target
    async fn scroll(
        &self,
        ctx: &ExecCtx,
        target: &crate::types::ScrollTarget,
        behavior: crate::types::ScrollBehavior,
    ) -> Result<ActionReport, ActionError>;

    /// Wait for condition
    async fn wait_for(
        &self,
        ctx: &ExecCtx,
        condition: &crate::types::WaitCondition,
        timeout_ms: u64,
    ) -> Result<ActionReport, ActionError>;
}

/// Default implementation of action primitives
pub struct DefaultActionPrimitives {
    /// CDP adapter for browser communication
    adapter: Arc<CdpAdapter>,

    /// Wait strategy for built-in waiting
    wait_strategy: Arc<dyn WaitStrategy>,

    /// Tracks adapter start state
    adapter_ready: OnceCell<()>,

    /// Cached execution contexts per route (session/page/frame)
    route_contexts: DashMap<String, Arc<ResolvedExecutionContext>>,

    /// Resolver used to turn anchors into actionable selectors
    anchor_resolver: Arc<dyn AnchorResolver>,
}

impl DefaultActionPrimitives {
    /// Create a new primitives implementation
    pub fn new(adapter: Arc<CdpAdapter>, wait_strategy: Arc<dyn WaitStrategy>) -> Self {
        Self::with_anchor_resolver(
            adapter,
            wait_strategy,
            Arc::new(ScriptAnchorResolver::default()),
        )
    }

    /// Create a primitives implementation with a custom anchor resolver
    pub fn with_anchor_resolver(
        adapter: Arc<CdpAdapter>,
        wait_strategy: Arc<dyn WaitStrategy>,
        anchor_resolver: Arc<dyn AnchorResolver>,
    ) -> Self {
        Self {
            adapter,
            wait_strategy,
            adapter_ready: OnceCell::new(),
            route_contexts: DashMap::new(),
            anchor_resolver,
        }
    }

    /// Get reference to CDP adapter
    pub fn adapter(&self) -> &Arc<CdpAdapter> {
        &self.adapter
    }

    /// Get reference to wait strategy
    pub fn wait_strategy(&self) -> &Arc<dyn WaitStrategy> {
        &self.wait_strategy
    }

    /// Resolve anchor into selector using the configured resolver
    pub async fn resolve_anchor_selector(
        &self,
        ctx: &ExecCtx,
        anchor: &AnchorDescriptor,
    ) -> Result<ResolvedSelector, ActionError> {
        self.anchor_resolver.resolve(self, ctx, anchor).await
    }

    /// Ensure the underlying adapter is started
    pub async fn ensure_adapter_ready(&self) -> Result<(), ActionError> {
        self.adapter_ready
            .get_or_try_init(|| async {
                Arc::clone(&self.adapter).start().await.map_err(|err| {
                    ActionError::Internal(format!("Failed to start CDP adapter: {}", err))
                })
            })
            .await
            .map(|_| ())
    }

    fn context_cache_key(route: &soulbrowser_core_types::ExecRoute) -> String {
        format!("{}::{}::{}", route.session.0, route.page.0, route.frame.0)
    }

    fn cleanup_context_cache(&self) {
        let active_pages: HashSet<cdp_adapter::PageId> = self
            .adapter
            .registry()
            .iter()
            .into_iter()
            .map(|(page, _)| page)
            .collect();

        if active_pages.is_empty() {
            return;
        }

        self.route_contexts
            .retain(|_, ctx| active_pages.contains(&ctx.page));
    }

    pub async fn resolve_route_context(
        &self,
        route: &soulbrowser_core_types::ExecRoute,
    ) -> Result<Arc<ResolvedExecutionContext>, ActionError> {
        let key = Self::context_cache_key(route);
        if let Some(existing) = self.route_contexts.get(&key) {
            return Ok(existing.value().clone());
        }

        self.ensure_adapter_ready().await?;
        self.cleanup_context_cache();

        let resolved = self
            .adapter
            .resolve_execution_context(route)
            .await
            .map_err(|err| ActionError::CdpIo(err.to_string()))?;
        let context = Arc::new(resolved);
        self.route_contexts.insert(key, context.clone());
        Ok(context)
    }

    pub async fn resolve_context(
        &self,
        ctx: &ExecCtx,
    ) -> Result<Arc<ResolvedExecutionContext>, ActionError> {
        self.resolve_route_context(&ctx.route).await
    }

    /// Capture URL/title signals for observability, logging errors but not failing actions.
    pub async fn capture_page_signals(&self, ctx: &ExecCtx) -> Result<PostSignals, ActionError> {
        let context = self.resolve_context(ctx).await?;
        let script =
            "(() => ({ url: window.location.href || null, title: document.title || null }))()";
        let value = self
            .adapter
            .evaluate_script_in_context(&context, script)
            .await
            .map_err(|err| ActionError::CdpIo(err.to_string()))?;

        let mut signals = PostSignals::default();
        signals.url_after = value
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        signals.title_after = value
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Ok(signals)
    }
}

#[async_trait]
impl ActionPrimitives for DefaultActionPrimitives {
    async fn navigate(
        &self,
        ctx: &ExecCtx,
        url: &str,
        wait_tier: crate::types::WaitTier,
    ) -> Result<ActionReport, ActionError> {
        navigate::execute_navigate(self, ctx, url, wait_tier).await
    }

    async fn click(
        &self,
        ctx: &ExecCtx,
        anchor: &crate::types::AnchorDescriptor,
        wait_tier: crate::types::WaitTier,
    ) -> Result<ActionReport, ActionError> {
        click::execute_click(self, ctx, anchor, wait_tier).await
    }

    async fn type_text(
        &self,
        ctx: &ExecCtx,
        anchor: &crate::types::AnchorDescriptor,
        text: &str,
        submit: bool,
        wait_tier: Option<crate::types::WaitTier>,
    ) -> Result<ActionReport, ActionError> {
        type_text::execute_type_text(self, ctx, anchor, text, submit, wait_tier).await
    }

    async fn select(
        &self,
        ctx: &ExecCtx,
        anchor: &crate::types::AnchorDescriptor,
        method: crate::types::SelectMethod,
        item: &str,
        wait_tier: crate::types::WaitTier,
    ) -> Result<ActionReport, ActionError> {
        select::execute_select(self, ctx, anchor, method, item, wait_tier).await
    }

    async fn scroll(
        &self,
        ctx: &ExecCtx,
        target: &crate::types::ScrollTarget,
        behavior: crate::types::ScrollBehavior,
    ) -> Result<ActionReport, ActionError> {
        scroll::execute_scroll(self, ctx, target, behavior).await
    }

    async fn wait_for(
        &self,
        ctx: &ExecCtx,
        condition: &crate::types::WaitCondition,
        timeout_ms: u64,
    ) -> Result<ActionReport, ActionError> {
        wait::execute_wait(self, ctx, condition, timeout_ms).await
    }
}
