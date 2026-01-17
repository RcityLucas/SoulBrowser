//! Prompt templates for agent loop mode.
//!
//! Contains system prompts and formatters that instruct the LLM
//! on how to interact with the browser state and make decisions.

use super::types::{AgentHistoryEntry, BrowserStateSummary};
use crate::model::AgentRequest;

/// Default system prompt for agent loop mode.
pub const AGENT_LOOP_SYSTEM_PROMPT: &str = r#"You are a browser automation agent. Your task is to interact with web pages to accomplish the user's goal through an iterative observe-think-act loop.

## How It Works
Each step you receive:
1. **Current browser state**: URL, title, scroll position
2. **Interactive elements**: Indexed element tree showing clickable/typeable elements
3. **Screenshot** (if enabled): Visual representation of the page
4. **Previous actions history**: What you did before and the results

You analyze this information and decide what action(s) to take next.

## Input Format

### Element Tree
Interactive elements are shown with indices in brackets:
```
[0]<button class="submit">Submit</button>
  [1]<span>Icon</span>
[2]<input type="text" placeholder="Search...">
[3]<a href="/about">About Us</a>
```

- **Indentation** shows parent-child relationships
- **Only elements with [index]** are interactive - you can click/type on them
- Text without brackets is non-interactive context

### History Format
Previous steps show:
- **Evaluation**: Did the last action achieve its goal?
- **Memory**: Important facts to remember
- **Actions**: What was attempted
- **Result**: Success or failure with details

## Available Actions

### Navigation
- **navigate**: Go directly to a URL
  ```json
  {"action": "navigate", "url": "https://www.google.com"}
  ```
  - ALWAYS use navigate to visit URLs, never type URLs into search boxes
  - Use for direct navigation when you know the target URL

### Element Interactions
- **click**: Click an element by index
  ```json
  {"action": "click", "element_index": 5}
  ```

- **type_text**: Type into an input field
  ```json
  {"action": "type_text", "element_index": 3, "text": "search query", "submit": true}
  ```
  - Set `submit: true` to press Enter after typing (useful for search forms)

- **select**: Choose from a dropdown
  ```json
  {"action": "select", "element_index": 7, "value": "option_value"}
  ```

### Page Control
- **scroll**: Scroll the page
  ```json
  {"action": "scroll", "direction": "Down", "amount": 500}
  ```
  - Directions: "Up", "Down", "Left", "Right"
  - Use when content is below/above current viewport

- **wait**: Wait for page to load/update
  ```json
  {"action": "wait", "ms": 1000}
  ```

### Task Completion
- **done**: Signal task completion
  ```json
  {"action": "done", "done_success": true, "done_text": "Found the weather: 25°C sunny"}
  ```
  - `done_success: true` - Task completed successfully
  - `done_success: false` - Task failed or impossible
  - MUST be the only action when used (never combine with other actions)

## Response Format

You MUST respond with valid JSON in this exact format:

```json
{
  "thinking": "Analyze current state, what I see, what needs to happen",
  "evaluation_previous_goal": "Success/Failed/Unknown - assessment of last action",
  "memory": "Key facts to remember: target URL, important element indices, progress made",
  "next_goal": "Immediate objective for this step",
  "actions": [
    {"action": "click", "element_index": 5}
  ]
}
```

### Field Requirements
- **thinking**: Your reasoning about the current situation (1-3 sentences)
- **evaluation_previous_goal**: How well did the last action work? (Skip on first step)
- **memory**: Critical information to carry forward (1-3 sentences, be specific about indices and values)
- **next_goal**: What you're trying to achieve with this step's actions
- **actions**: Array of 1-3 actions to execute

## Rules

### Screenshot Rules (when provided)
- **Screenshot is ground truth** - trust what you see over element tree when they conflict
- Use screenshot to verify element visibility and page state
- Check for loading indicators, popups, or overlays that might block interaction

### Element Interaction Rules
- **Only use indices from current element tree** - indices change after page updates
- If target element not found, scroll to find it or navigate to correct page
- For forms: fill fields before clicking submit
- For search: use `submit: true` with type_text instead of separate click

### Multi-Action Efficiency
Combine related actions in one step (max 3 actions):
- ✅ Good: `type_text` with `submit: true` for search
- ✅ Good: `scroll` then `click` if element might be below viewport
- ❌ Bad: More than 3 actions in one step
- ❌ Bad: `done` combined with other actions

### Task Completion Rules
Call `done` when:
- ✅ Task is fully completed and verified
- ✅ Requested information has been found and can be reported
- ✅ Task is impossible (page doesn't exist, login required, etc.)

Do NOT call `done`:
- ❌ Before verifying the action worked
- ❌ Combined with other actions
- ❌ When you're unsure and should try more steps

### Error Recovery
If an action fails:
1. Check if page state changed unexpectedly
2. Look for the element with a different index (page may have reloaded)
3. Try scrolling to find the element
4. Try an alternative approach
5. Only give up after 2-3 attempts

## Examples

### Example 1: Search on Google
```json
{
  "thinking": "I'm on Google's homepage. I see a search input at index 2 and need to search for 'weather Beijing'",
  "evaluation_previous_goal": "Success - navigated to Google successfully",
  "memory": "Task: search for weather. Search input is [2]. Will type and submit.",
  "next_goal": "Enter search query and submit",
  "actions": [
    {"action": "type_text", "element_index": 2, "text": "weather Beijing", "submit": true}
  ]
}
```

### Example 2: Navigate to specific site
```json
{
  "thinking": "User wants to open Baidu. I should navigate directly to the URL.",
  "memory": "Task: open Baidu homepage",
  "next_goal": "Navigate to Baidu",
  "actions": [
    {"action": "navigate", "url": "https://www.baidu.com"}
  ]
}
```

### Example 3: Task completed
```json
{
  "thinking": "Search results show weather information. Beijing weather is 15°C, cloudy.",
  "evaluation_previous_goal": "Success - search results loaded with weather info",
  "memory": "Found weather: Beijing 15°C cloudy",
  "next_goal": "Report the result",
  "actions": [
    {"action": "done", "done_success": true, "done_text": "Beijing weather: 15°C, cloudy"}
  ]
}
```

### Example 4: Handling failure
```json
{
  "thinking": "Login page appeared unexpectedly. Cannot proceed without credentials.",
  "evaluation_previous_goal": "Failed - redirected to login page instead of content",
  "memory": "Blocked by login requirement. No credentials available.",
  "next_goal": "Report inability to complete task",
  "actions": [
    {"action": "done", "done_success": false, "done_text": "Cannot complete: login required but no credentials provided"}
  ]
}
```
"#;

/// Prompt for vision-enabled mode with screenshot support.
pub const VISION_PROMPT_ADDITION: &str = r#"
## Screenshot Analysis
You have been provided with a screenshot of the current page. Use visual information to:
- Verify element positions and visibility
- Identify UI elements that may not be in the element tree
- Understand the overall page layout and context
- Detect loading states, overlays, or visual issues

When the element tree is insufficient, describe what you see in the screenshot to guide your actions.
"#;

/// Formats the complete system prompt for agent loop mode.
pub fn format_system_prompt(enable_vision: bool) -> String {
    let mut prompt = AGENT_LOOP_SYSTEM_PROMPT.to_string();
    if enable_vision {
        prompt.push_str(VISION_PROMPT_ADDITION);
    }
    prompt
}

/// Formats the user message containing the task and browser state.
pub fn format_user_message(
    request: &AgentRequest,
    state: &BrowserStateSummary,
    history: &[AgentHistoryEntry],
) -> String {
    let mut message = String::new();

    // Task description
    message.push_str("## Task\n");
    message.push_str(&request.goal);
    message.push('\n');

    // Add intent context if available
    if let Some(ref primary_goal) = request.intent.primary_goal {
        if primary_goal != &request.goal {
            message.push_str("\nPrimary goal: ");
            message.push_str(primary_goal);
            message.push('\n');
        }
    }

    // Step info
    let current_step = history.len() + 1;
    message.push_str(&format!("\n## Step Info\nCurrent step: {}\n", current_step));

    // History summary if there are previous steps (show last 10 steps for context)
    if !history.is_empty() {
        message.push_str("\n## Previous Actions\n");
        let max_history = 10;
        let start_idx = history.len().saturating_sub(max_history);

        for entry in history.iter().skip(start_idx) {
            message.push_str(&format!("\n### Step {}\n", entry.step_number));

            // Evaluation of previous goal
            if let Some(ref eval) = entry.evaluation {
                message.push_str(&format!("Evaluation: {}\n", eval));
            }

            // Memory
            if let Some(ref mem) = entry.memory {
                message.push_str(&format!("Memory: {}\n", mem));
            }

            // State summary
            message.push_str(&format!("Page: {}\n", entry.state_summary));

            // Actions taken
            if entry.actions_taken.is_empty() {
                message.push_str("Actions: (none)\n");
            } else {
                let actions_str: Vec<String> = entry
                    .actions_taken
                    .iter()
                    .map(|a| {
                        let action_name = format!("{:?}", a.action_type).to_lowercase();
                        if let Some(idx) = a.element_index {
                            format!("{}[{}]", action_name, idx)
                        } else if let Some(ref url) = a.params.url {
                            format!("{}({})", action_name, truncate_string(url, 50))
                        } else if let Some(ref text) = a.params.text {
                            format!("{}(\"{}\")", action_name, truncate_string(text, 30))
                        } else {
                            action_name
                        }
                    })
                    .collect();
                message.push_str(&format!("Actions: {}\n", actions_str.join(", ")));
            }

            // Result
            if entry.result.success {
                message.push_str("Result: ✓ Success\n");
            } else {
                let error_msg = entry
                    .result
                    .error_message
                    .as_deref()
                    .unwrap_or("unknown error");
                message.push_str(&format!("Result: ✗ Failed - {}\n", error_msg));
            }
        }
    }

    // Current browser state
    message.push_str("\n## Current Browser State\n");
    message.push_str(&format!("URL: {}\n", state.url));
    if let Some(ref title) = state.title {
        message.push_str(&format!("Title: {}\n", title));
    }

    // Scroll position with page context
    let scroll = &state.scroll_position;
    if scroll.total_height > 0 {
        let percentage = scroll.scroll_percentage() as i32;
        let viewport = scroll.viewport_height;
        let total = scroll.total_height;

        // Calculate pages above and below
        let pages_above = if viewport > 0 {
            scroll.pixels_from_top / viewport.max(1)
        } else {
            0
        };
        let remaining_height = total - scroll.pixels_from_top - viewport;
        let pages_below = if viewport > 0 && remaining_height > 0 {
            remaining_height / viewport.max(1)
        } else {
            0
        };

        message.push_str(&format!(
            "Scroll: {}% | {} page(s) above, {} page(s) below\n",
            percentage, pages_above, pages_below
        ));
    }

    // Element tree
    message.push_str(&format!("\n## Interactive Elements ({} total)\n", state.element_count));
    message.push_str(&state.element_tree);
    message.push('\n');

    // Screenshot indicator
    if state.screenshot_base64.is_some() {
        message.push_str("\n[Screenshot attached - use it as ground truth for element visibility]\n");
    }

    message
}

/// Truncate a string to max length with ellipsis.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

/// Formats a compact state update for subsequent turns.
pub fn format_state_update(
    state: &BrowserStateSummary,
    last_action_result: Option<&str>,
) -> String {
    let mut message = String::new();

    if let Some(result) = last_action_result {
        message.push_str("## Last Action Result\n");
        message.push_str(result);
        message.push_str("\n\n");
    }

    message.push_str("## Updated Browser State\n");
    message.push_str(&format!("URL: {}\n", state.url));
    if let Some(ref title) = state.title {
        message.push_str(&format!("Title: {}\n", title));
    }

    let scroll = &state.scroll_position;
    if scroll.total_height > 0 {
        message.push_str(&format!(
            "Scroll: {}% ({}/{} pixels)\n",
            scroll.scroll_percentage() as i32,
            scroll.pixels_from_top,
            scroll.total_height
        ));
    }

    message.push_str(&format!("\nElement count: {}\n", state.element_count));
    message.push_str("\n## Interactive Elements\n");
    message.push_str(&state.element_tree);

    if state.screenshot_base64.is_some() {
        message.push_str("\n\n[Screenshot attached]");
    }

    message
}

/// Action type constants for prompt documentation.
pub mod action_types {
    /// Navigate to a URL.
    pub const GO_TO_URL: &str = "go_to_url";
    /// Click an element by index.
    pub const CLICK_ELEMENT: &str = "click_element";
    /// Type text into an input element.
    pub const INPUT_TEXT: &str = "input_text";
    /// Select an option from a dropdown.
    pub const SELECT_OPTION: &str = "select_option";
    /// Scroll down the page.
    pub const SCROLL_DOWN: &str = "scroll_down";
    /// Scroll up the page.
    pub const SCROLL_UP: &str = "scroll_up";
    /// Scroll an element into view.
    pub const SCROLL_TO_ELEMENT: &str = "scroll_to_element";
    /// Wait for a duration.
    pub const WAIT: &str = "wait";
    /// Signal task completion.
    pub const DONE: &str = "done";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_loop::types::ScrollPosition;
    use soulbrowser_core_types::TaskId;
    use std::collections::HashMap;

    fn test_request() -> AgentRequest {
        AgentRequest::new(TaskId::new(), "Search for 'rust programming' on Google")
    }

    fn test_state() -> BrowserStateSummary {
        BrowserStateSummary {
            url: "https://www.google.com".to_string(),
            title: Some("Google".to_string()),
            element_tree:
                "[0]<input type=\"text\" aria-label=\"Search\">\n[1]<button>Google Search</button>"
                    .to_string(),
            selector_map: HashMap::new(),
            screenshot_base64: None,
            scroll_position: ScrollPosition::default(),
            focused_element: None,
            element_count: 2,
        }
    }

    #[test]
    fn test_format_system_prompt_without_vision() {
        let prompt = format_system_prompt(false);
        assert!(prompt.contains("browser automation agent"));
        assert!(prompt.contains("Element Tree Format"));
        assert!(prompt.contains("Available Actions"));
        assert!(!prompt.contains("Screenshot Analysis"));
    }

    #[test]
    fn test_format_system_prompt_with_vision() {
        let prompt = format_system_prompt(true);
        assert!(prompt.contains("browser automation agent"));
        assert!(prompt.contains("Screenshot Analysis"));
    }

    #[test]
    fn test_format_user_message() {
        let request = test_request();
        let state = test_state();
        let history: Vec<AgentHistoryEntry> = vec![];

        let message = format_user_message(&request, &state, &history);

        assert!(message.contains("## Task"));
        assert!(message.contains("Search for 'rust programming' on Google"));
        assert!(message.contains("URL: https://www.google.com"));
        assert!(message.contains("Title: Google"));
        assert!(message.contains("## Interactive Elements"));
        assert!(message.contains("[0]<input"));
    }

    #[test]
    fn test_format_state_update() {
        let state = test_state();
        let update = format_state_update(&state, Some("Click successful"));

        assert!(update.contains("## Last Action Result"));
        assert!(update.contains("Click successful"));
        assert!(update.contains("## Updated Browser State"));
        assert!(update.contains("URL: https://www.google.com"));
    }

    #[test]
    fn test_format_state_update_without_result() {
        let state = test_state();
        let update = format_state_update(&state, None);

        assert!(!update.contains("## Last Action Result"));
        assert!(update.contains("## Updated Browser State"));
    }
}
