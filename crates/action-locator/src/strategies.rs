//! Element resolution strategies
//!
//! Three strategies in fallback order:
//! 1. CSS - Direct CSS selector resolution
//! 2. ARIA/AX - Accessibility tree matching
//! 3. Text - Text content matching

use crate::{errors::LocatorError, types::*};
use action_primitives::AnchorDescriptor;
use async_trait::async_trait;
use cdp_adapter::CdpAdapter;
use perceiver_structural::errors::PerceiverError;
use perceiver_structural::{
    AnchorDescriptor as StructuralAnchorDescriptor, AnchorResolution, ResolveHint, ResolveOptions,
    StructuralPerceiver,
};
use soulbrowser_core_types::ExecRoute;
use std::sync::Arc;
use tracing::debug;
use tracing::warn;

const STUB_REASON: &str =
    "Action locator is stubbed; wire it to the real browser and build without the 'stub' feature";

fn stubbed_strategy_error(strategy: LocatorStrategy) -> LocatorError {
    LocatorError::StrategyFailed {
        strategy: strategy.name().to_string(),
        reason: STUB_REASON.to_string(),
    }
}

/// Strategy trait for element resolution
#[async_trait]
pub trait Strategy: Send + Sync {
    /// Attempt to resolve element using this strategy
    async fn resolve(
        &self,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<Vec<Candidate>, LocatorError>;

    /// Get strategy type
    fn strategy_type(&self) -> LocatorStrategy;

    /// Get strategy name
    fn name(&self) -> &'static str {
        self.strategy_type().name()
    }
}

/// CSS selector resolution strategy
pub struct CssStrategy {
    _adapter: Arc<CdpAdapter>,
    perceiver: Arc<dyn StructuralPerceiver>,
}

impl CssStrategy {
    /// Create a new CSS strategy
    pub fn new(adapter: Arc<CdpAdapter>, perceiver: Arc<dyn StructuralPerceiver>) -> Self {
        Self {
            _adapter: adapter,
            perceiver,
        }
    }
}

#[async_trait]
impl Strategy for CssStrategy {
    async fn resolve(
        &self,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<Vec<Candidate>, LocatorError> {
        match anchor {
            AnchorDescriptor::Css(selector) => {
                debug!("Resolving CSS selector: {}", selector);
                self.resolve_css_selector(selector, anchor, route).await
            }
            _ => {
                // Not a CSS anchor, return empty
                Ok(Vec::new())
            }
        }
    }

    fn strategy_type(&self) -> LocatorStrategy {
        LocatorStrategy::Css
    }
}

impl CssStrategy {
    /// Resolve element by CSS selector
    async fn resolve_css_selector(
        &self,
        selector: &str,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<Vec<Candidate>, LocatorError> {
        if crate::is_stubbed() {
            return Err(stubbed_strategy_error(LocatorStrategy::Css));
        }
        if selector.is_empty() {
            return Err(LocatorError::InvalidAnchor(
                "Empty CSS selector".to_string(),
            ));
        }

        debug!("CSS resolution for selector: {}", selector);

        let hint = ResolveHint::Css(selector.to_string());
        resolve_with_structural(&self.perceiver, route, hint, anchor, LocatorStrategy::Css).await
    }
}

/// ARIA/AX accessibility tree resolution strategy
pub struct AriaAxStrategy {
    perceiver: Arc<dyn StructuralPerceiver>,
}

impl AriaAxStrategy {
    /// Create a new ARIA/AX strategy
    pub fn new(perceiver: Arc<dyn StructuralPerceiver>) -> Self {
        Self { perceiver }
    }
}

#[async_trait]
impl Strategy for AriaAxStrategy {
    async fn resolve(
        &self,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<Vec<Candidate>, LocatorError> {
        match anchor {
            AnchorDescriptor::Aria { role, name } => {
                debug!("Resolving ARIA role={}, name={}", role, name);
                self.resolve_aria_attributes(role, name, anchor, route)
                    .await
            }
            AnchorDescriptor::Css(selector) => {
                // Fallback: try to find similar elements by ARIA
                debug!("Attempting ARIA fallback for CSS selector: {}", selector);
                self.find_aria_fallback_for_css(selector, anchor, route)
                    .await
            }
            AnchorDescriptor::Text { content, .. } => {
                // Fallback: try to find by accessible name
                debug!("Attempting ARIA fallback for text: {}", content);
                self.find_aria_fallback_for_text(content, anchor, route)
                    .await
            }
        }
    }

    fn strategy_type(&self) -> LocatorStrategy {
        LocatorStrategy::AriaAx
    }
}

impl AriaAxStrategy {
    /// Resolve element by ARIA role and accessible name
    async fn resolve_aria_attributes(
        &self,
        role: &str,
        name: &str,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<Vec<Candidate>, LocatorError> {
        if crate::is_stubbed() {
            return Err(stubbed_strategy_error(LocatorStrategy::AriaAx));
        }
        if role.is_empty() {
            return Err(LocatorError::InvalidAnchor("Empty ARIA role".to_string()));
        }

        let trimmed_name = name.trim();
        if trimmed_name.is_empty() {
            return Err(LocatorError::InvalidAnchor(
                "Empty ARIA accessible name".to_string(),
            ));
        }

        debug!(
            "ARIA/AX resolution for role={}, name={}",
            role, trimmed_name
        );

        let hint = ResolveHint::Aria {
            role: role.to_string(),
            name: Some(trimmed_name.to_string()),
        };

        resolve_with_structural(
            &self.perceiver,
            route,
            hint,
            anchor,
            LocatorStrategy::AriaAx,
        )
        .await
    }

    /// Find ARIA fallback for failed CSS selector
    async fn find_aria_fallback_for_css(
        &self,
        selector: &str,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<Vec<Candidate>, LocatorError> {
        if crate::is_stubbed() {
            return Err(stubbed_strategy_error(LocatorStrategy::AriaAx));
        }
        let mut candidates = Vec::new();
        if let Some(role) = infer_role_from_selector(selector) {
            let keywords = extract_keywords_from_selector(selector);
            for keyword in keywords {
                match self
                    .resolve_aria_attributes(role, &keyword, anchor, route)
                    .await
                {
                    Ok(mut matches) if !matches.is_empty() => {
                        candidates.append(&mut matches);
                    }
                    Ok(_) => {}
                    Err(err) => warn!("ARIA fallback failed: {}", err),
                }
            }
        }
        Ok(candidates)
    }

    /// Find ARIA fallback for failed text match
    async fn find_aria_fallback_for_text(
        &self,
        content: &str,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<Vec<Candidate>, LocatorError> {
        if crate::is_stubbed() {
            return Err(stubbed_strategy_error(LocatorStrategy::AriaAx));
        }
        let mut candidates = Vec::new();
        for role in ["button", "link", "menuitem", "textbox"] {
            match self
                .resolve_aria_attributes(role, content, anchor, route)
                .await
            {
                Ok(mut matches) if !matches.is_empty() => {
                    candidates.append(&mut matches);
                }
                Ok(_) => {}
                Err(err) => debug!("ARIA fallback ({}) failed: {}", role, err),
            }
        }
        Ok(candidates)
    }
}

/// Text content matching strategy
pub struct TextStrategy {
    perceiver: Arc<dyn StructuralPerceiver>,
}

impl TextStrategy {
    /// Create a new text strategy
    pub fn new(perceiver: Arc<dyn StructuralPerceiver>) -> Self {
        Self { perceiver }
    }
}

#[async_trait]
impl Strategy for TextStrategy {
    async fn resolve(
        &self,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<Vec<Candidate>, LocatorError> {
        match anchor {
            AnchorDescriptor::Text { content, exact } => {
                debug!("Resolving text content: {} (exact={})", content, exact);
                self.resolve_text_content(content, *exact, anchor, route)
                    .await
            }
            AnchorDescriptor::Css(selector) => {
                // Fallback: extract semantic meaning from selector
                debug!("Attempting text fallback for CSS selector: {}", selector);
                self.find_text_fallback_for_css(selector, anchor, route)
                    .await
            }
            AnchorDescriptor::Aria { name, .. } => {
                // Fallback: use ARIA name as text content
                debug!("Attempting text fallback for ARIA name: {}", name);
                self.resolve_text_content(name, false, anchor, route).await
            }
        }
    }

    fn strategy_type(&self) -> LocatorStrategy {
        LocatorStrategy::Text
    }
}

impl TextStrategy {
    /// Resolve element by text content
    async fn resolve_text_content(
        &self,
        content: &str,
        exact: bool,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<Vec<Candidate>, LocatorError> {
        if crate::is_stubbed() {
            return Err(stubbed_strategy_error(LocatorStrategy::Text));
        }
        if content.is_empty() {
            return Err(LocatorError::InvalidAnchor(
                "Empty text content".to_string(),
            ));
        }

        debug!("Text resolution for content: {} (exact={})", content, exact);

        let mut pattern = content.to_string();
        if exact {
            pattern = pattern.trim().to_string();
        }
        let hint = ResolveHint::Text { pattern };

        resolve_with_structural(&self.perceiver, route, hint, anchor, LocatorStrategy::Text).await
    }

    /// Find text fallback for failed CSS selector
    async fn find_text_fallback_for_css(
        &self,
        selector: &str,
        anchor: &AnchorDescriptor,
        route: &ExecRoute,
    ) -> Result<Vec<Candidate>, LocatorError> {
        if crate::is_stubbed() {
            return Err(stubbed_strategy_error(LocatorStrategy::Text));
        }
        debug!("Text fallback for CSS selector: {}", selector);

        let mut results = Vec::new();
        for keyword in extract_keywords_from_selector(selector) {
            match self
                .resolve_text_content(&keyword, false, anchor, route)
                .await
            {
                Ok(mut matches) if !matches.is_empty() => {
                    results.append(&mut matches);
                }
                Ok(_) => {}
                Err(err) => debug!("Text fallback failed for '{}': {}", keyword, err),
            }
        }

        Ok(results)
    }
}

/// Extract semantic keywords from CSS selector
fn extract_keywords_from_selector(selector: &str) -> Vec<String> {
    let mut keywords = Vec::new();

    // Remove CSS syntax characters
    let cleaned = selector
        .replace(['#', '.', '>', '+', '~', '[', ']'], " ")
        .replace('-', " ")
        .replace('_', " ");

    // Split into words and collect non-trivial ones
    for word in cleaned.split_whitespace() {
        if word.len() > 2 && !is_html_tag(word) {
            keywords.push(word.to_lowercase());
        }
    }

    keywords
}

/// Check if string is a common HTML tag
fn is_html_tag(s: &str) -> bool {
    matches!(
        s.to_lowercase().as_str(),
        "div" | "span" | "button" | "input" | "a" | "p" | "h1" | "h2" | "h3" | "ul" | "li" | "form"
    )
}

async fn resolve_with_structural(
    perceiver: &Arc<dyn StructuralPerceiver>,
    route: &ExecRoute,
    hint: ResolveHint,
    anchor: &AnchorDescriptor,
    strategy: LocatorStrategy,
) -> Result<Vec<Candidate>, LocatorError> {
    let resolution = perceiver
        .resolve_anchor(route.clone(), hint, resolve_options())
        .await
        .map_err(|e| map_perceiver_error(strategy, e))?;
    Ok(convert_resolution(resolution, strategy, anchor))
}

fn resolve_options() -> ResolveOptions {
    ResolveOptions {
        max_candidates: 5,
        fuzziness: None,
        debounce_ms: None,
    }
}

fn convert_resolution(
    resolution: AnchorResolution,
    strategy: LocatorStrategy,
    anchor: &AnchorDescriptor,
) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    candidates.push(candidate_from_descriptor(
        &resolution.primary,
        strategy,
        anchor,
    ));

    for descriptor in resolution.candidates {
        candidates.push(candidate_from_descriptor(&descriptor, strategy, anchor));
    }

    candidates
}

fn candidate_from_descriptor(
    descriptor: &StructuralAnchorDescriptor,
    strategy: LocatorStrategy,
    anchor: &AnchorDescriptor,
) -> Candidate {
    let mut candidate = Candidate::new(
        descriptor_element_id(descriptor),
        strategy,
        descriptor.confidence as f64,
        anchor.clone(),
    );
    candidate.metadata = metadata_from_descriptor(descriptor);
    candidate
}

fn descriptor_element_id(descriptor: &StructuralAnchorDescriptor) -> String {
    if let Some(id) = descriptor.backend_node_id {
        return format!("backend-node-{}", id);
    }
    if let Some(geometry) = &descriptor.geometry {
        return format!(
            "geom-{}-{}-{}-{}",
            geometry.x, geometry.y, geometry.width, geometry.height
        );
    }
    if let Some(obj) = descriptor.value.as_object() {
        if let Some(node_id) = obj.get("nodeId").and_then(|v| v.as_u64()) {
            return format!("node-{}", node_id);
        }
        if let Some(selector) = obj.get("selector").and_then(|v| v.as_str()) {
            return selector.to_string();
        }
    }
    descriptor.strategy.clone()
}

fn metadata_from_descriptor(descriptor: &StructuralAnchorDescriptor) -> CandidateMetadata {
    let mut metadata = CandidateMetadata::default();
    if let Some(obj) = descriptor.value.as_object() {
        metadata.tag_name = obj
            .get("tagName")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        metadata.visible_text = obj
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        metadata.aria_role = obj
            .get("ariaRole")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        metadata.aria_label = obj
            .get("ariaLabel")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        metadata.dom_index = obj
            .get("domIndex")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        metadata.is_visible = obj
            .get("visible")
            .and_then(|v| v.as_bool())
            .unwrap_or(metadata.is_visible);
        metadata.is_enabled = obj
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(metadata.is_enabled);
    }
    metadata
}

fn map_perceiver_error(strategy: LocatorStrategy, err: PerceiverError) -> LocatorError {
    LocatorError::StrategyFailed {
        strategy: strategy.name().to_string(),
        reason: err.to_string(),
    }
}

fn infer_role_from_selector(selector: &str) -> Option<&'static str> {
    let lower = selector.to_ascii_lowercase();
    if lower.contains("btn") || lower.contains("button") {
        Some("button")
    } else if lower.contains("link") {
        Some("link")
    } else if lower.contains("menu") {
        Some("menuitem")
    } else if lower.contains("input") || lower.contains("field") {
        Some("textbox")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_keywords() {
        let keywords = extract_keywords_from_selector("#submit-action");
        assert!(keywords.contains(&"submit".to_string()));
        assert!(keywords.contains(&"action".to_string()));

        let keywords = extract_keywords_from_selector(".user_login_form");
        assert!(keywords.contains(&"user".to_string()));
        assert!(keywords.contains(&"login".to_string()));
        // "form" is filtered out as HTML tag
        assert!(!keywords.contains(&"form".to_string()));
    }

    #[test]
    fn test_is_html_tag() {
        assert!(is_html_tag("div"));
        assert!(is_html_tag("button"));
        assert!(!is_html_tag("submit"));
        assert!(!is_html_tag("login"));
    }

    #[test]
    fn test_locator_strategy() {
        assert_eq!(LocatorStrategy::Css.name(), "css");
        assert_eq!(LocatorStrategy::AriaAx.name(), "aria-ax");
        assert_eq!(LocatorStrategy::Text.name(), "text");
    }

    #[test]
    fn test_fallback_chain() {
        let chain = LocatorStrategy::fallback_chain();
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0], LocatorStrategy::Css);
        assert_eq!(chain[1], LocatorStrategy::AriaAx);
        assert_eq!(chain[2], LocatorStrategy::Text);
    }
}
