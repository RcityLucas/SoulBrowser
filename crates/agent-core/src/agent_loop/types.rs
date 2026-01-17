//! Core data types for the agent loop execution mode.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Browser state snapshot formatted for LLM consumption.
///
/// This structure contains all the information the LLM needs to make
/// decisions about the next action to take.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserStateSummary {
    /// Current page URL.
    pub url: String,

    /// Page title (from document.title).
    pub title: Option<String>,

    /// Indexed interactive elements in tree format.
    /// Example: "[0]<button>Submit</button>\n[1]<input type=\"text\">"
    pub element_tree: String,

    /// Mapping from element index to selector information.
    /// Used to resolve LLM's element references to actual DOM targets.
    #[serde(default)]
    pub selector_map: HashMap<u32, ElementSelectorRef>,

    /// Base64-encoded screenshot (if vision is enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot_base64: Option<String>,

    /// Current scroll position information.
    pub scroll_position: ScrollPosition,

    /// Index of currently focused element (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused_element: Option<u32>,

    /// Number of indexed elements.
    pub element_count: u32,
}

/// Reference to element selector information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementSelectorRef {
    /// CSS selector (if available).
    pub css_selector: Option<String>,

    /// CDP backend node ID for direct element access.
    pub backend_node_id: Option<i64>,

    /// ARIA-based selector.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aria_selector: Option<AriaSelector>,

    /// Text content of the element (truncated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>,

    /// Element tag name.
    pub tag_name: String,
}

/// ARIA-based element selector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AriaSelector {
    /// ARIA role (e.g., "button", "textbox").
    pub role: String,
    /// Accessible name.
    pub name: String,
}

/// Scroll position information.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScrollPosition {
    /// Pixels scrolled from top.
    pub pixels_from_top: i32,
    /// Total scrollable height.
    pub total_height: i32,
    /// Viewport height.
    pub viewport_height: i32,
}

impl ScrollPosition {
    /// Calculate scroll percentage.
    pub fn scroll_percentage(&self) -> f32 {
        if self.total_height <= self.viewport_height {
            100.0
        } else {
            let scrollable = self.total_height - self.viewport_height;
            (self.pixels_from_top as f32 / scrollable as f32 * 100.0).min(100.0)
        }
    }

    /// Check if at bottom of page.
    pub fn is_at_bottom(&self) -> bool {
        self.scroll_percentage() >= 95.0
    }
}

/// LLM output for a single agent loop iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOutput {
    /// Chain-of-thought reasoning about current state.
    #[serde(default)]
    pub thinking: String,

    /// Evaluation of whether the previous action achieved its goal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation_previous_goal: Option<String>,

    /// Important facts to remember for future steps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,

    /// Immediate next objective.
    #[serde(default)]
    pub next_goal: String,

    /// Actions to execute (typically 1-3).
    #[serde(default)]
    pub actions: Vec<AgentAction>,
}

impl AgentOutput {
    /// Check if output contains a done action.
    pub fn is_done(&self) -> bool {
        self.actions
            .iter()
            .any(|a| matches!(a.action_type, AgentActionType::Done))
    }

    /// Extract done action result if present.
    pub fn done_result(&self) -> Option<(&bool, &str)> {
        for action in &self.actions {
            if let AgentActionType::Done = action.action_type {
                if let (Some(success), Some(text)) =
                    (&action.params.done_success, &action.params.done_text)
                {
                    return Some((success, text.as_str()));
                }
            }
        }
        None
    }
}

/// Single action in agent output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAction {
    /// Action type.
    #[serde(rename = "action")]
    pub action_type: AgentActionType,

    /// Element index for element-targeting actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_index: Option<u32>,

    /// Additional parameters based on action type.
    #[serde(flatten)]
    pub params: AgentActionParams,
}

/// Supported action types in agent loop mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentActionType {
    /// Navigate to URL.
    Navigate,
    /// Click an element.
    Click,
    /// Type text into an element.
    TypeText,
    /// Select option from dropdown.
    Select,
    /// Scroll the page.
    Scroll,
    /// Wait for a condition or duration.
    Wait,
    /// Signal task completion.
    Done,
}

/// Action parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentActionParams {
    /// URL for navigate action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Text for type_text action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    /// Whether to submit after typing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submit: Option<bool>,

    /// Value for select action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    /// Scroll direction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<ScrollDirection>,

    /// Scroll amount in pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<i32>,

    /// Wait duration in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,

    /// Done action: success flag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done_success: Option<bool>,

    /// Done action: completion text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done_text: Option<String>,

    /// Done action: success flag (alternative field name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
}

/// Scroll direction for scroll actions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
    ToElement,
}

/// Entry in agent loop history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHistoryEntry {
    /// Step number (1-indexed).
    pub step_number: u32,

    /// Brief summary of browser state at this step.
    pub state_summary: String,

    /// Actions taken at this step.
    pub actions_taken: Vec<AgentAction>,

    /// Result of action execution.
    pub result: AgentActionResult,

    /// LLM's thinking for this step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,

    /// LLM's next goal for this step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_goal: Option<String>,

    /// LLM's evaluation of the previous step's goal achievement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<String>,

    /// LLM's memory/notes to carry forward to future steps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

impl AgentHistoryEntry {
    /// Create a new history entry.
    pub fn new(
        step_number: u32,
        state_summary: String,
        actions: Vec<AgentAction>,
        result: AgentActionResult,
    ) -> Self {
        Self {
            step_number,
            state_summary,
            actions_taken: actions,
            result,
            thinking: None,
            next_goal: None,
            evaluation: None,
            memory: None,
        }
    }

    /// Create a new history entry with full LLM output context.
    pub fn from_output(
        step_number: u32,
        state_summary: String,
        output: &super::types::AgentOutput,
        result: AgentActionResult,
    ) -> Self {
        Self {
            step_number,
            state_summary,
            actions_taken: output.actions.clone(),
            result,
            thinking: Some(output.thinking.clone()),
            next_goal: Some(output.next_goal.clone()),
            evaluation: output.evaluation_previous_goal.clone(),
            memory: output.memory.clone(),
        }
    }

    /// Create an error entry.
    pub fn error(step_number: u32, error: String) -> Self {
        Self {
            step_number,
            state_summary: "Error occurred".to_string(),
            actions_taken: Vec::new(),
            result: AgentActionResult {
                success: false,
                error_message: Some(error),
                state_changed: false,
            },
            thinking: None,
            next_goal: None,
            evaluation: None,
            memory: None,
        }
    }

    /// Get a brief summary of actions taken.
    pub fn actions_summary(&self) -> String {
        self.actions_taken
            .iter()
            .map(|a| format!("{:?}", a.action_type))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Result of action execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentActionResult {
    /// Whether all actions succeeded.
    pub success: bool,

    /// Error message if any action failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,

    /// Whether the page state changed after actions.
    pub state_changed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_output_is_done() {
        let output = AgentOutput {
            thinking: "Task complete".to_string(),
            evaluation_previous_goal: None,
            memory: None,
            next_goal: "Finish".to_string(),
            actions: vec![AgentAction {
                action_type: AgentActionType::Done,
                element_index: None,
                params: AgentActionParams {
                    done_success: Some(true),
                    done_text: Some("Completed successfully".to_string()),
                    ..Default::default()
                },
            }],
        };

        assert!(output.is_done());
        let (success, text) = output.done_result().unwrap();
        assert!(success);
        assert_eq!(text, "Completed successfully");
    }

    #[test]
    fn test_scroll_position_percentage() {
        let pos = ScrollPosition {
            pixels_from_top: 500,
            total_height: 2000,
            viewport_height: 1000,
        };

        assert!((pos.scroll_percentage() - 50.0).abs() < 0.1);
        assert!(!pos.is_at_bottom());

        let at_bottom = ScrollPosition {
            pixels_from_top: 950,
            total_height: 2000,
            viewport_height: 1000,
        };
        assert!(at_bottom.is_at_bottom());
    }

    #[test]
    fn test_agent_action_serialization() {
        let action = AgentAction {
            action_type: AgentActionType::Click,
            element_index: Some(5),
            params: AgentActionParams::default(),
        };

        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"action\":\"click\""));
        assert!(json.contains("\"element_index\":5"));
    }
}
