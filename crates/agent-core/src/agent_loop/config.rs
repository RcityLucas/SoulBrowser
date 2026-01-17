//! Configuration for agent loop execution mode.

use serde::{Deserialize, Serialize};

/// Configuration for the agent loop (observe-think-act) execution mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLoopConfig {
    /// Maximum iterations before forcing stop.
    /// Default: 100
    pub max_steps: u32,

    /// Maximum actions per step (like browser-use's max_actions_per_step).
    /// Typically 1-3 actions per LLM decision.
    /// Default: 3
    pub max_actions_per_step: u32,

    /// Consecutive failures before aborting the loop.
    /// Default: 3
    pub max_consecutive_failures: u32,

    /// Whether to capture and send screenshots to LLM (vision mode).
    /// Default: true
    pub enable_vision: bool,

    /// Maximum number of elements to index for LLM.
    /// Higher values provide more context but increase token usage.
    /// Default: 500
    pub max_elements: u32,

    /// Timeout per action execution in milliseconds.
    /// Default: 30000 (30 seconds)
    pub action_timeout_ms: u64,

    /// Timeout for LLM API calls in milliseconds.
    /// Default: 60000 (60 seconds)
    pub llm_timeout_ms: u64,

    /// Step-level timeout in milliseconds (entire observe-think-act cycle).
    /// Default: 180000 (3 minutes)
    pub step_timeout_ms: u64,

    /// Whether to include element attributes in the tree representation.
    /// Default: true
    pub include_element_attributes: bool,

    /// Maximum depth of DOM tree to traverse.
    /// Default: 50
    pub max_dom_depth: u32,

    /// Whether to enable extended thinking mode for complex tasks.
    /// Default: false
    pub use_extended_thinking: bool,

    /// Minimum wait between actions in milliseconds.
    /// Default: 100
    pub wait_between_actions_ms: u64,

    /// Whether to automatically retry failed LLM calls.
    /// Default: true
    pub retry_llm_on_empty_response: bool,

    /// Maximum text length per element in the tree.
    /// Default: 100
    pub max_element_text_length: u32,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            max_steps: 100,
            max_actions_per_step: 3,
            max_consecutive_failures: 3,
            enable_vision: true,
            max_elements: 500,
            action_timeout_ms: 30_000,
            llm_timeout_ms: 60_000,
            step_timeout_ms: 180_000,
            include_element_attributes: true,
            max_dom_depth: 50,
            use_extended_thinking: false,
            wait_between_actions_ms: 100,
            retry_llm_on_empty_response: true,
            max_element_text_length: 100,
        }
    }
}

impl AgentLoopConfig {
    /// Create a new config with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a minimal config for testing.
    pub fn minimal() -> Self {
        Self {
            max_steps: 10,
            max_actions_per_step: 1,
            max_consecutive_failures: 2,
            enable_vision: false,
            max_elements: 100,
            action_timeout_ms: 5_000,
            llm_timeout_ms: 10_000,
            step_timeout_ms: 30_000,
            include_element_attributes: false,
            max_dom_depth: 20,
            use_extended_thinking: false,
            wait_between_actions_ms: 50,
            retry_llm_on_empty_response: false,
            max_element_text_length: 50,
        }
    }

    /// Create a config optimized for fast execution.
    pub fn fast() -> Self {
        Self {
            max_steps: 50,
            max_actions_per_step: 3,
            max_consecutive_failures: 2,
            enable_vision: false,
            max_elements: 200,
            action_timeout_ms: 15_000,
            llm_timeout_ms: 30_000,
            step_timeout_ms: 60_000,
            include_element_attributes: true,
            max_dom_depth: 30,
            use_extended_thinking: false,
            wait_between_actions_ms: 50,
            retry_llm_on_empty_response: true,
            max_element_text_length: 80,
        }
    }

    /// Create a config for vision-enabled execution with screenshots.
    pub fn with_vision() -> Self {
        Self {
            enable_vision: true,
            ..Self::default()
        }
    }

    /// Builder: set max steps.
    pub fn max_steps(mut self, steps: u32) -> Self {
        self.max_steps = steps;
        self
    }

    /// Builder: set vision mode.
    pub fn vision(mut self, enabled: bool) -> Self {
        self.enable_vision = enabled;
        self
    }

    /// Builder: set max actions per step.
    pub fn actions_per_step(mut self, count: u32) -> Self {
        self.max_actions_per_step = count;
        self
    }

    /// Builder: set max elements.
    pub fn elements(mut self, count: u32) -> Self {
        self.max_elements = count;
        self
    }

    /// Builder: set LLM timeout.
    pub fn llm_timeout(mut self, ms: u64) -> Self {
        self.llm_timeout_ms = ms;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AgentLoopConfig::default();
        assert_eq!(config.max_steps, 100);
        assert_eq!(config.max_actions_per_step, 3);
        assert!(config.enable_vision);
    }

    #[test]
    fn test_builder() {
        let config = AgentLoopConfig::new()
            .max_steps(50)
            .vision(false)
            .actions_per_step(2);

        assert_eq!(config.max_steps, 50);
        assert!(!config.enable_vision);
        assert_eq!(config.max_actions_per_step, 2);
    }

    #[test]
    fn test_minimal_config() {
        let config = AgentLoopConfig::minimal();
        assert_eq!(config.max_steps, 10);
        assert!(!config.enable_vision);
    }
}
