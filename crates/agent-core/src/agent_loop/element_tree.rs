//! Element tree builder for creating indexed DOM representation.
//!
//! Transforms raw DOM/AX snapshots into browser-use style indexed elements
//! that can be consumed by the LLM for decision making.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::types::AriaSelector;

/// Interactive element tags that should be indexed.
const INTERACTIVE_TAGS: &[&str] = &[
    "a", "button", "input", "select", "textarea", "option", "label", "summary", "details",
];

/// Tags with implicit interactivity based on attributes.
const POTENTIALLY_INTERACTIVE_TAGS: &[&str] =
    &["div", "span", "li", "tr", "td", "th", "img", "svg", "path"];

/// Attributes that indicate interactivity.
const INTERACTIVE_ATTRIBUTES: &[&str] = &[
    "onclick",
    "onmousedown",
    "onmouseup",
    "ontouchstart",
    "role",
    "tabindex",
    "contenteditable",
    "draggable",
];

/// ARIA roles that indicate interactivity.
const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "checkbox",
    "radio",
    "textbox",
    "combobox",
    "listbox",
    "option",
    "menuitem",
    "tab",
    "switch",
    "slider",
    "spinbutton",
    "searchbox",
    "gridcell",
    "treeitem",
];

/// Builder for creating indexed element trees from DOM snapshots.
#[derive(Debug, Clone)]
pub struct ElementTreeBuilder {
    /// Maximum number of elements to index.
    max_elements: u32,
    /// Maximum depth to traverse.
    max_depth: u32,
    /// Whether to include element attributes.
    include_attributes: bool,
    /// Maximum text length per element.
    max_text_length: u32,
}

impl Default for ElementTreeBuilder {
    fn default() -> Self {
        Self {
            max_elements: 500,
            max_depth: 50,
            include_attributes: true,
            max_text_length: 100,
        }
    }
}

impl ElementTreeBuilder {
    /// Create a new builder with specified max elements.
    pub fn new(max_elements: u32) -> Self {
        Self {
            max_elements,
            ..Default::default()
        }
    }

    /// Set maximum traversal depth.
    pub fn with_max_depth(mut self, depth: u32) -> Self {
        self.max_depth = depth;
        self
    }

    /// Set whether to include attributes.
    pub fn with_attributes(mut self, include: bool) -> Self {
        self.include_attributes = include;
        self
    }

    /// Set maximum text length per element.
    pub fn with_max_text_length(mut self, len: u32) -> Self {
        self.max_text_length = len;
        self
    }

    /// Build indexed element tree from DOM snapshot.
    ///
    /// # Arguments
    /// * `dom_raw` - Raw DOM snapshot JSON from CDP DOMSnapshot.captureSnapshot
    /// * `ax_raw` - Raw accessibility tree JSON from CDP Accessibility.getFullAXTree
    pub fn build(&self, dom_raw: &Value, ax_raw: &Value) -> ElementTreeResult {
        let mut elements = Vec::new();
        let mut selector_map = HashMap::new();
        let mut index = 0u32;

        // Parse DOM snapshot format
        // CDP DOMSnapshot format has: documents[], strings[]
        if let Some(documents) = dom_raw.get("documents").and_then(Value::as_array) {
            let strings = dom_raw
                .get("strings")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .map(|v| v.as_str().unwrap_or("").to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            for doc in documents {
                self.process_document(
                    doc,
                    &strings,
                    ax_raw,
                    &mut elements,
                    &mut selector_map,
                    &mut index,
                );
            }
        } else {
            // Fallback: try simpler node-based format
            self.process_simple_dom(dom_raw, &mut elements, &mut selector_map, &mut index);
        }

        ElementTreeResult {
            tree_string: self.format_tree(&elements),
            selector_map,
            element_count: index,
        }
    }

    /// Process a document from CDP DOMSnapshot format.
    fn process_document(
        &self,
        doc: &Value,
        strings: &[String],
        ax_raw: &Value,
        elements: &mut Vec<IndexedElement>,
        selector_map: &mut HashMap<u32, ElementSelectorRef>,
        index: &mut u32,
    ) {
        // Extract node arrays
        let node_names = doc.get("nodes").and_then(|n| n.get("nodeName"));
        let node_types = doc.get("nodes").and_then(|n| n.get("nodeType"));
        let node_values = doc.get("nodes").and_then(|n| n.get("nodeValue"));
        let parent_indices = doc.get("nodes").and_then(|n| n.get("parentIndex"));
        let attributes = doc.get("nodes").and_then(|n| n.get("attributes"));
        let backend_node_ids = doc.get("nodes").and_then(|n| n.get("backendNodeId"));
        let text_values = doc.get("textBoxes").and_then(|t| t.get("content"));

        // Build AX tree index for role/name lookup
        let ax_index = self.build_ax_index(ax_raw);

        // Process nodes
        if let (Some(names), Some(types)) = (
            node_names.and_then(Value::as_array),
            node_types.and_then(Value::as_array),
        ) {
            let node_count = names.len();
            let mut depths = vec![0u32; node_count];

            // Calculate depths
            if let Some(parents) = parent_indices.and_then(Value::as_array) {
                for (i, parent) in parents.iter().enumerate() {
                    if let Some(p) = parent.as_i64() {
                        if p >= 0 && (p as usize) < node_count {
                            depths[i] = depths[p as usize] + 1;
                        }
                    }
                }
            }

            for i in 0..node_count {
                if *index >= self.max_elements {
                    break;
                }

                let node_type = types.get(i).and_then(Value::as_i64).unwrap_or(0);
                if node_type != 1 {
                    // Only process element nodes
                    continue;
                }

                let depth = depths[i];
                if depth > self.max_depth {
                    continue;
                }

                // Get tag name
                let name_idx = names.get(i).and_then(Value::as_i64).unwrap_or(-1);
                let tag_name = if name_idx >= 0 && (name_idx as usize) < strings.len() {
                    strings[name_idx as usize].to_lowercase()
                } else {
                    continue;
                };

                // Get backend node ID
                let backend_node_id = backend_node_ids
                    .and_then(Value::as_array)
                    .and_then(|arr| arr.get(i))
                    .and_then(Value::as_i64);

                // Get attributes
                let attrs = self.extract_attributes(attributes, i, strings);

                // Check if interactive
                if !self.is_interactive(&tag_name, &attrs) {
                    continue;
                }

                // Get text content
                let text_content = self.extract_text_content(node_values, text_values, i, strings);

                // Get ARIA info from AX tree
                let aria_info = backend_node_id.and_then(|id| ax_index.get(&id).cloned());

                // Build HTML representation
                let html_repr =
                    self.build_html_repr(&tag_name, &attrs, &text_content, aria_info.as_ref());

                // Add to results
                elements.push(IndexedElement {
                    index: *index,
                    html_repr,
                    backend_node_id,
                    tag_name: tag_name.clone(),
                    depth,
                    parent_index: None, // TODO: track parent in future enhancement
                });

                selector_map.insert(
                    *index,
                    ElementSelectorRef {
                        css_selector: self.build_css_selector(&tag_name, &attrs),
                        backend_node_id,
                        aria_selector: aria_info.map(|ai| AriaSelector {
                            role: ai.role,
                            name: ai.name,
                        }),
                        text_content: if text_content.is_empty() {
                            None
                        } else {
                            Some(text_content)
                        },
                        tag_name,
                    },
                );

                *index += 1;
            }
        }
    }

    /// Process simpler DOM format (fallback).
    fn process_simple_dom(
        &self,
        dom: &Value,
        elements: &mut Vec<IndexedElement>,
        selector_map: &mut HashMap<u32, ElementSelectorRef>,
        index: &mut u32,
    ) {
        self.traverse_node(dom, elements, selector_map, index, 0);
    }

    /// Recursively traverse DOM nodes.
    fn traverse_node(
        &self,
        node: &Value,
        elements: &mut Vec<IndexedElement>,
        selector_map: &mut HashMap<u32, ElementSelectorRef>,
        index: &mut u32,
        depth: u32,
    ) {
        if *index >= self.max_elements || depth > self.max_depth {
            return;
        }

        // Handle node object format
        if let Some(tag) = node.get("nodeName").and_then(Value::as_str) {
            let tag_name = tag.to_lowercase();
            let attrs = self.extract_simple_attributes(node);
            let backend_node_id = node.get("backendNodeId").and_then(Value::as_i64);

            if self.is_interactive(&tag_name, &attrs) {
                let text_content = node
                    .get("textContent")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();

                let html_repr = self.build_html_repr(&tag_name, &attrs, &text_content, None);

                elements.push(IndexedElement {
                    index: *index,
                    html_repr,
                    backend_node_id,
                    tag_name: tag_name.clone(),
                    depth,
                    parent_index: None, // TODO: track parent in future enhancement
                });

                selector_map.insert(
                    *index,
                    ElementSelectorRef {
                        css_selector: self.build_css_selector(&tag_name, &attrs),
                        backend_node_id,
                        aria_selector: None,
                        text_content: if text_content.is_empty() {
                            None
                        } else {
                            Some(text_content)
                        },
                        tag_name,
                    },
                );

                *index += 1;
            }

            // Process children
            if let Some(children) = node.get("children").and_then(Value::as_array) {
                for child in children {
                    self.traverse_node(child, elements, selector_map, index, depth + 1);
                }
            }
        }
    }

    /// Build AX tree index for quick lookups by backend node ID.
    fn build_ax_index(&self, ax_raw: &Value) -> HashMap<i64, AxNodeInfo> {
        let mut index = HashMap::new();

        if let Some(nodes) = ax_raw.get("nodes").and_then(Value::as_array) {
            for node in nodes {
                let backend_id = node.get("backendDOMNodeId").and_then(Value::as_i64);
                let role = node
                    .get("role")
                    .and_then(|r| r.get("value"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let name = node
                    .get("name")
                    .and_then(|n| n.get("value"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();

                if let Some(id) = backend_id {
                    if !role.is_empty() {
                        index.insert(id, AxNodeInfo { role, name });
                    }
                }
            }
        }

        index
    }

    /// Extract attributes from CDP DOMSnapshot format.
    fn extract_attributes(
        &self,
        attributes: Option<&Value>,
        node_index: usize,
        strings: &[String],
    ) -> HashMap<String, String> {
        let mut result = HashMap::new();

        if let Some(attrs_array) = attributes.and_then(Value::as_array) {
            if let Some(node_attrs) = attrs_array.get(node_index).and_then(Value::as_array) {
                for chunk in node_attrs.chunks(2) {
                    if chunk.len() == 2 {
                        let key_idx = chunk[0].as_i64().unwrap_or(-1);
                        let val_idx = chunk[1].as_i64().unwrap_or(-1);

                        if key_idx >= 0
                            && val_idx >= 0
                            && (key_idx as usize) < strings.len()
                            && (val_idx as usize) < strings.len()
                        {
                            let key = strings[key_idx as usize].clone();
                            let value = strings[val_idx as usize].clone();
                            result.insert(key, value);
                        }
                    }
                }
            }
        }

        result
    }

    /// Extract attributes from simple node format.
    fn extract_simple_attributes(&self, node: &Value) -> HashMap<String, String> {
        let mut result = HashMap::new();

        if let Some(attrs) = node.get("attributes").and_then(Value::as_object) {
            for (key, value) in attrs {
                if let Some(v) = value.as_str() {
                    result.insert(key.clone(), v.to_string());
                }
            }
        }

        result
    }

    /// Extract text content for a node.
    fn extract_text_content(
        &self,
        node_values: Option<&Value>,
        text_values: Option<&Value>,
        node_index: usize,
        strings: &[String],
    ) -> String {
        // Try node value first
        if let Some(values) = node_values.and_then(Value::as_array) {
            if let Some(val_idx) = values.get(node_index).and_then(Value::as_i64) {
                if val_idx >= 0 && (val_idx as usize) < strings.len() {
                    let text = &strings[val_idx as usize];
                    if !text.is_empty() {
                        return self.truncate_text(text);
                    }
                }
            }
        }

        // Fallback to text box content if available
        if let Some(contents) = text_values.and_then(Value::as_array) {
            // This is simplified; real implementation would need layout index matching
            if let Some(first) = contents.first() {
                if let Some(idx) = first.as_i64() {
                    if idx >= 0 && (idx as usize) < strings.len() {
                        return self.truncate_text(&strings[idx as usize]);
                    }
                }
            }
        }

        String::new()
    }

    /// Check if an element is interactive.
    fn is_interactive(&self, tag_name: &str, attrs: &HashMap<String, String>) -> bool {
        // Check tag-based interactivity
        if INTERACTIVE_TAGS.contains(&tag_name.as_ref()) {
            return true;
        }

        // Check attribute-based interactivity
        for attr in INTERACTIVE_ATTRIBUTES {
            if attrs.contains_key(*attr) {
                if *attr == "tabindex" {
                    let val = attrs.get(*attr).map(|s| s.as_str()).unwrap_or("-1");
                    if val == "-1" {
                        continue;
                    }
                }
                return true;
            }
        }

        // Check role-based interactivity
        if let Some(role) = attrs.get("role") {
            if INTERACTIVE_ROLES.contains(&role.to_lowercase().as_str()) {
                return true;
            }
        }

        // Check potentially interactive tags with click handlers
        if POTENTIALLY_INTERACTIVE_TAGS.contains(&tag_name.as_ref()) {
            if attrs.contains_key("onclick")
                || attrs.contains_key("data-action")
                || attrs
                    .get("class")
                    .map(|c| c.contains("btn"))
                    .unwrap_or(false)
            {
                return true;
            }
        }

        false
    }

    /// Build HTML representation for an element.
    fn build_html_repr(
        &self,
        tag_name: &str,
        attrs: &HashMap<String, String>,
        text_content: &str,
        aria_info: Option<&AxNodeInfo>,
    ) -> String {
        let mut parts = vec![format!("<{}", tag_name)];

        // Add key attributes
        if self.include_attributes {
            let important_attrs = [
                "id",
                "class",
                "type",
                "name",
                "value",
                "placeholder",
                "href",
                "title",
                "aria-label",
            ];

            for attr in &important_attrs {
                if let Some(value) = attrs.get(*attr) {
                    let truncated = self.truncate_text(value);
                    if !truncated.is_empty() {
                        parts.push(format!(" {}=\"{}\"", attr, self.escape_html(&truncated)));
                    }
                }
            }

            // Add ARIA role if from AX tree
            if let Some(info) = aria_info {
                if !info.role.is_empty() && !attrs.contains_key("role") {
                    parts.push(format!(" role=\"{}\"", info.role));
                }
            }
        }

        parts.push(">".to_string());

        // Add text content
        let content = if !text_content.is_empty() {
            self.truncate_text(text_content)
        } else if let Some(info) = aria_info {
            if !info.name.is_empty() {
                self.truncate_text(&info.name)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if !content.is_empty() {
            parts.push(self.escape_html(&content));
        }

        parts.push(format!("</{}>", tag_name));

        parts.join("")
    }

    /// Build CSS selector for an element.
    fn build_css_selector(
        &self,
        tag_name: &str,
        attrs: &HashMap<String, String>,
    ) -> Option<String> {
        // Prefer ID
        if let Some(id) = attrs.get("id") {
            if !id.is_empty() && !id.contains(' ') {
                return Some(format!("#{}", id));
            }
        }

        // Build selector with tag and key attributes
        let mut parts = vec![tag_name.to_string()];

        if let Some(name) = attrs.get("name") {
            if !name.is_empty() {
                parts.push(format!("[name=\"{}\"]", name));
            }
        } else if let Some(class) = attrs.get("class") {
            // Use first meaningful class
            if let Some(first_class) = class.split_whitespace().next() {
                if !first_class.is_empty() {
                    parts.push(format!(".{}", first_class));
                }
            }
        }

        if let Some(type_attr) = attrs.get("type") {
            parts.push(format!("[type=\"{}\"]", type_attr));
        }

        Some(parts.join(""))
    }

    /// Truncate text to max length (character-based, not byte-based for UTF-8 safety).
    fn truncate_text(&self, text: &str) -> String {
        let trimmed = text.trim();
        let char_count = trimmed.chars().count();
        let max_chars = self.max_text_length as usize;
        if char_count <= max_chars {
            trimmed.to_string()
        } else {
            // Take max_chars - 3 characters to leave room for "..."
            let truncated: String = trimmed.chars().take(max_chars.saturating_sub(3)).collect();
            format!("{}...", truncated)
        }
    }

    /// Escape HTML special characters.
    fn escape_html(&self, text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    /// Format elements into tree string with hierarchical indentation.
    fn format_tree(&self, elements: &[IndexedElement]) -> String {
        if elements.is_empty() {
            return String::new();
        }

        // Find minimum depth to use as base level
        let min_depth = elements.iter().map(|e| e.depth).min().unwrap_or(0);

        elements
            .iter()
            .map(|e| {
                // Calculate relative depth for indentation
                let relative_depth = e.depth.saturating_sub(min_depth);
                let indent = "  ".repeat(relative_depth as usize);
                format!("{}[{}]{}", indent, e.index, e.html_repr)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Result of building an element tree.
#[derive(Debug, Clone)]
pub struct ElementTreeResult {
    /// Formatted tree string for LLM consumption.
    pub tree_string: String,
    /// Mapping from index to selector info.
    pub selector_map: HashMap<u32, ElementSelectorRef>,
    /// Total number of indexed elements.
    pub element_count: u32,
}

/// Internal structure for indexed elements.
#[derive(Debug, Clone)]
struct IndexedElement {
    index: u32,
    html_repr: String,
    backend_node_id: Option<i64>,
    tag_name: String,
    depth: u32,
    parent_index: Option<usize>,
}

/// ARIA node info extracted from AX tree.
#[derive(Debug, Clone)]
struct AxNodeInfo {
    role: String,
    name: String,
}

/// Selector info for element lookup (re-export for external use).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementSelector {
    /// CSS selector.
    pub css_selector: Option<String>,
    /// Backend node ID.
    pub backend_node_id: Option<i64>,
    /// ARIA selector.
    pub aria_selector: Option<AriaSelector>,
}

/// Internal selector reference type used during tree building.
/// This is the same as types::ElementSelectorRef but defined locally
/// to avoid circular dependencies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementSelectorRef {
    /// CSS selector (if available).
    pub css_selector: Option<String>,
    /// CDP backend node ID for direct element access.
    pub backend_node_id: Option<i64>,
    /// ARIA-based selector.
    pub aria_selector: Option<AriaSelector>,
    /// Text content of the element (truncated).
    pub text_content: Option<String>,
    /// Element tag name.
    pub tag_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_is_interactive() {
        let builder = ElementTreeBuilder::default();

        // Button should be interactive
        assert!(builder.is_interactive("button", &HashMap::new()));

        // Input should be interactive
        assert!(builder.is_interactive("input", &HashMap::new()));

        // Plain div should not be interactive
        assert!(!builder.is_interactive("div", &HashMap::new()));

        // Div with onclick should be interactive
        let mut attrs = HashMap::new();
        attrs.insert("onclick".to_string(), "handler()".to_string());
        assert!(builder.is_interactive("div", &attrs));

        // Div with role=button should be interactive
        let mut attrs = HashMap::new();
        attrs.insert("role".to_string(), "button".to_string());
        assert!(builder.is_interactive("div", &attrs));
    }

    #[test]
    fn test_build_html_repr() {
        let builder = ElementTreeBuilder::default();

        let mut attrs = HashMap::new();
        attrs.insert("type".to_string(), "submit".to_string());
        attrs.insert("class".to_string(), "btn primary".to_string());

        let repr = builder.build_html_repr("button", &attrs, "Click me", None);

        assert!(repr.contains("<button"));
        assert!(repr.contains("type=\"submit\""));
        assert!(repr.contains("Click me"));
        assert!(repr.contains("</button>"));
    }

    #[test]
    fn test_build_css_selector() {
        let builder = ElementTreeBuilder::default();

        // With ID
        let mut attrs = HashMap::new();
        attrs.insert("id".to_string(), "submit-btn".to_string());
        assert_eq!(
            builder.build_css_selector("button", &attrs),
            Some("#submit-btn".to_string())
        );

        // With name
        let mut attrs = HashMap::new();
        attrs.insert("name".to_string(), "email".to_string());
        attrs.insert("type".to_string(), "text".to_string());
        let selector = builder.build_css_selector("input", &attrs).unwrap();
        assert!(selector.contains("input"));
        assert!(selector.contains("[name=\"email\"]"));
    }

    #[test]
    fn test_truncate_text() {
        let builder = ElementTreeBuilder::new(500).with_max_text_length(20);

        assert_eq!(builder.truncate_text("Short"), "Short");
        assert_eq!(
            builder.truncate_text("This is a very long text that should be truncated"),
            "This is a very lo..."
        );
    }

    #[test]
    fn test_simple_dom_processing() {
        let builder = ElementTreeBuilder::new(100);

        let dom = json!({
            "nodeName": "BUTTON",
            "nodeType": 1,
            "textContent": "Submit",
            "attributes": {
                "type": "submit",
                "class": "btn"
            },
            "backendNodeId": 123
        });

        let result = builder.build(&dom, &json!({}));

        assert_eq!(result.element_count, 1);
        assert!(result.tree_string.contains("[0]"));
        assert!(result.tree_string.contains("<button"));
        assert!(result.selector_map.contains_key(&0));
    }
}
