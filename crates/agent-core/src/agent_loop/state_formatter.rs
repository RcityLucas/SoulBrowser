//! Browser state formatter for LLM consumption.
//!
//! Transforms perception data into the BrowserStateSummary format
//! that can be sent to the LLM for decision making.

use serde_json::Value;

use super::config::AgentLoopConfig;
use super::element_tree::ElementTreeBuilder;
use super::types::{BrowserStateSummary, ScrollPosition};

/// Formats browser state for LLM consumption.
#[derive(Debug, Clone)]
pub struct StateFormatter {
    element_builder: ElementTreeBuilder,
    enable_vision: bool,
}

impl StateFormatter {
    /// Create a new state formatter.
    pub fn new(config: &AgentLoopConfig) -> Self {
        Self {
            element_builder: ElementTreeBuilder::new(config.max_elements)
                .with_max_depth(config.max_dom_depth)
                .with_attributes(config.include_element_attributes)
                .with_max_text_length(config.max_element_text_length),
            enable_vision: config.enable_vision,
        }
    }

    /// Create formatter with custom element builder.
    pub fn with_element_builder(element_builder: ElementTreeBuilder, enable_vision: bool) -> Self {
        Self {
            element_builder,
            enable_vision,
        }
    }

    /// Format browser state from raw DOM/AX data.
    ///
    /// # Arguments
    /// * `dom_raw` - Raw DOM snapshot from CDP
    /// * `ax_raw` - Raw accessibility tree from CDP
    /// * `url` - Current page URL
    /// * `title` - Page title (optional)
    /// * `screenshot_base64` - Base64-encoded screenshot (optional)
    /// * `scroll_info` - Scroll position information (optional)
    pub fn format_state(
        &self,
        dom_raw: &Value,
        ax_raw: &Value,
        url: &str,
        title: Option<&str>,
        screenshot_base64: Option<&str>,
        scroll_info: Option<ScrollPosition>,
    ) -> BrowserStateSummary {
        // Build element tree
        let tree_result = self.element_builder.build(dom_raw, ax_raw);

        // Convert selector_map types
        let selector_map = tree_result
            .selector_map
            .into_iter()
            .map(|(k, v)| (k, v.into()))
            .collect();

        BrowserStateSummary {
            url: url.to_string(),
            title: title.map(|s| s.to_string()),
            element_tree: tree_result.tree_string,
            selector_map,
            screenshot_base64: if self.enable_vision {
                screenshot_base64.map(|s| s.to_string())
            } else {
                None
            },
            scroll_position: scroll_info.unwrap_or_default(),
            focused_element: None,
            element_count: tree_result.element_count,
        }
    }

    /// Format state from a more structured perception result.
    ///
    /// This is a convenience method when you have structured perception data
    /// rather than raw JSON values.
    pub fn format_from_perception(&self, perception: &PerceptionData) -> BrowserStateSummary {
        self.format_state(
            &perception.dom_raw,
            &perception.ax_raw,
            &perception.url,
            perception.title.as_deref(),
            perception.screenshot_base64.as_deref(),
            perception.scroll_position.clone(),
        )
    }

    /// Create a minimal state summary for error scenarios.
    pub fn error_state(url: &str, error: &str) -> BrowserStateSummary {
        BrowserStateSummary {
            url: url.to_string(),
            title: Some(format!("Error: {}", error)),
            element_tree: format!("<!-- Error loading page: {} -->", error),
            selector_map: Default::default(),
            screenshot_base64: None,
            scroll_position: Default::default(),
            focused_element: None,
            element_count: 0,
        }
    }

    /// Check if vision is enabled.
    pub fn is_vision_enabled(&self) -> bool {
        self.enable_vision
    }
}

/// Structured perception data for formatting.
#[derive(Debug, Clone, Default)]
pub struct PerceptionData {
    /// Raw DOM snapshot JSON.
    pub dom_raw: Value,
    /// Raw accessibility tree JSON.
    pub ax_raw: Value,
    /// Current page URL.
    pub url: String,
    /// Page title.
    pub title: Option<String>,
    /// Base64-encoded screenshot.
    pub screenshot_base64: Option<String>,
    /// Scroll position.
    pub scroll_position: Option<ScrollPosition>,
}

impl PerceptionData {
    /// Create new perception data.
    pub fn new(dom_raw: Value, ax_raw: Value, url: impl Into<String>) -> Self {
        Self {
            dom_raw,
            ax_raw,
            url: url.into(),
            title: None,
            screenshot_base64: None,
            scroll_position: None,
        }
    }

    /// Set page title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set screenshot.
    pub fn with_screenshot(mut self, screenshot: impl Into<String>) -> Self {
        self.screenshot_base64 = Some(screenshot.into());
        self
    }

    /// Set scroll position.
    pub fn with_scroll(mut self, scroll: ScrollPosition) -> Self {
        self.scroll_position = Some(scroll);
        self
    }
}

/// Convert element_tree::ElementSelectorRef to types::ElementSelectorRef.
/// These are the same structure but defined in different modules to avoid
/// circular dependencies during compilation.
impl From<super::element_tree::ElementSelectorRef> for super::types::ElementSelectorRef {
    fn from(v: super::element_tree::ElementSelectorRef) -> Self {
        Self {
            css_selector: v.css_selector,
            backend_node_id: v.backend_node_id,
            aria_selector: v.aria_selector,
            text_content: v.text_content,
            tag_name: v.tag_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_config() -> AgentLoopConfig {
        AgentLoopConfig {
            max_elements: 100,
            max_dom_depth: 20,
            enable_vision: false,
            include_element_attributes: true,
            max_element_text_length: 50,
            ..Default::default()
        }
    }

    #[test]
    fn test_format_state_basic() {
        let formatter = StateFormatter::new(&test_config());

        let dom = json!({
            "nodeName": "BUTTON",
            "nodeType": 1,
            "textContent": "Click me",
            "attributes": {}
        });

        let state = formatter.format_state(
            &dom,
            &json!({}),
            "https://example.com",
            Some("Test Page"),
            None,
            None,
        );

        assert_eq!(state.url, "https://example.com");
        assert_eq!(state.title, Some("Test Page".to_string()));
        assert!(state.element_tree.contains("[0]"));
        assert!(state.screenshot_base64.is_none());
    }

    #[test]
    fn test_format_with_vision_disabled() {
        let config = AgentLoopConfig {
            enable_vision: false,
            ..test_config()
        };
        let formatter = StateFormatter::new(&config);

        let state = formatter.format_state(
            &json!({}),
            &json!({}),
            "https://example.com",
            None,
            Some("base64data"),
            None,
        );

        // Screenshot should be None even if provided
        assert!(state.screenshot_base64.is_none());
    }

    #[test]
    fn test_format_with_vision_enabled() {
        let config = AgentLoopConfig {
            enable_vision: true,
            ..test_config()
        };
        let formatter = StateFormatter::new(&config);

        let state = formatter.format_state(
            &json!({}),
            &json!({}),
            "https://example.com",
            None,
            Some("base64data"),
            None,
        );

        assert_eq!(state.screenshot_base64, Some("base64data".to_string()));
    }

    #[test]
    fn test_error_state() {
        let state = StateFormatter::error_state("https://example.com", "Connection refused");

        assert_eq!(state.url, "https://example.com");
        assert!(state.title.unwrap().contains("Error"));
        assert!(state.element_tree.contains("Connection refused"));
        assert_eq!(state.element_count, 0);
    }

    #[test]
    fn test_perception_data_builder() {
        let data = PerceptionData::new(json!({}), json!({}), "https://example.com")
            .with_title("Test")
            .with_screenshot("base64")
            .with_scroll(ScrollPosition {
                pixels_from_top: 100,
                total_height: 1000,
                viewport_height: 500,
            });

        assert_eq!(data.url, "https://example.com");
        assert_eq!(data.title, Some("Test".to_string()));
        assert!(data.screenshot_base64.is_some());
        assert!(data.scroll_position.is_some());
    }
}
