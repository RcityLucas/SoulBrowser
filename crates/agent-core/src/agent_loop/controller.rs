//! Agent Loop Controller - main orchestration for observe-think-act cycle.
//!
//! This module implements the browser-use style agent loop where the LLM
//! is consulted at each step to decide the next action based on current
//! browser state.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use super::config::AgentLoopConfig;
use super::types::{
    AgentAction, AgentActionResult, AgentActionType, AgentHistoryEntry, AgentOutput,
    BrowserStateSummary,
};

/// Result of an agent loop execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLoopResult {
    /// Final status of the loop.
    pub status: AgentLoopStatus,
    /// Completion or error message.
    pub message: String,
    /// Total steps taken.
    pub steps_taken: u32,
    /// Final output from the agent (if completed successfully).
    pub final_output: Option<String>,
    /// Execution history.
    pub history: Vec<AgentHistoryEntry>,
    /// Total execution time in milliseconds.
    pub total_time_ms: u64,
}

impl AgentLoopResult {
    /// Create a completed result.
    pub fn completed(
        message: String,
        steps: u32,
        history: Vec<AgentHistoryEntry>,
        time_ms: u64,
    ) -> Self {
        Self {
            status: AgentLoopStatus::Completed,
            final_output: Some(message.clone()),
            message,
            steps_taken: steps,
            history,
            total_time_ms: time_ms,
        }
    }

    /// Create a failed result.
    pub fn failed(
        message: String,
        steps: u32,
        history: Vec<AgentHistoryEntry>,
        time_ms: u64,
    ) -> Self {
        Self {
            status: AgentLoopStatus::Failed,
            message,
            steps_taken: steps,
            final_output: None,
            history,
            total_time_ms: time_ms,
        }
    }

    /// Create a max steps reached result.
    pub fn max_steps_reached(steps: u32, history: Vec<AgentHistoryEntry>, time_ms: u64) -> Self {
        Self {
            status: AgentLoopStatus::MaxStepsReached,
            message: format!("Reached maximum steps limit: {}", steps),
            steps_taken: steps,
            final_output: None,
            history,
            total_time_ms: time_ms,
        }
    }

    /// Create an in-progress placeholder (should not be returned as final result).
    pub fn in_progress() -> Self {
        Self {
            status: AgentLoopStatus::InProgress,
            message: "Loop in progress".to_string(),
            steps_taken: 0,
            final_output: None,
            history: Vec::new(),
            total_time_ms: 0,
        }
    }

    /// Check if the loop completed successfully.
    pub fn is_success(&self) -> bool {
        matches!(self.status, AgentLoopStatus::Completed)
    }
}

/// Status of the agent loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoopStatus {
    /// Task completed successfully.
    Completed,
    /// Task failed due to errors.
    Failed,
    /// Reached maximum steps without completion.
    MaxStepsReached,
    /// Loop is still in progress (internal state).
    InProgress,
    /// Loop was cancelled by user.
    Cancelled,
}

/// Result of a single step execution.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// History entry for this step.
    pub history_entry: AgentHistoryEntry,
    /// Whether the task is done.
    pub is_done: bool,
    /// Result if done.
    pub done_result: Option<DoneResult>,
}

/// Result from a done action.
#[derive(Debug, Clone)]
pub struct DoneResult {
    /// Whether the task succeeded.
    pub success: bool,
    /// Completion message.
    pub message: String,
}

/// Internal state of the agent loop.
#[derive(Debug, Default)]
struct LoopState {
    step_count: u32,
    consecutive_failures: u32,
    history: Vec<AgentHistoryEntry>,
    is_done: bool,
    is_cancelled: bool,
    final_result: Option<AgentLoopResult>,
}

/// Controller for the agent loop execution.
///
/// This is a generic controller that can work with any LLM provider
/// and action executor. The actual execution depends on the callbacks
/// provided during execution.
#[derive(Debug)]
pub struct AgentLoopController {
    config: AgentLoopConfig,
    state: Mutex<LoopState>,
    start_time: Mutex<Option<Instant>>,
}

impl AgentLoopController {
    /// Create a new controller with the given configuration.
    pub fn new(config: AgentLoopConfig) -> Self {
        Self {
            config,
            state: Mutex::new(LoopState::default()),
            start_time: Mutex::new(None),
        }
    }

    /// Create a controller with default configuration.
    pub fn default_config() -> Self {
        Self::new(AgentLoopConfig::default())
    }

    /// Get the configuration.
    pub fn config(&self) -> &AgentLoopConfig {
        &self.config
    }

    /// Cancel the loop.
    pub async fn cancel(&self) {
        let mut state = self.state.lock().await;
        state.is_cancelled = true;
    }

    /// Check if cancelled.
    pub async fn is_cancelled(&self) -> bool {
        let state = self.state.lock().await;
        state.is_cancelled
    }

    /// Get current step count.
    pub async fn step_count(&self) -> u32 {
        let state = self.state.lock().await;
        state.step_count
    }

    /// Get execution history.
    pub async fn history(&self) -> Vec<AgentHistoryEntry> {
        let state = self.state.lock().await;
        state.history.clone()
    }

    /// Reset the controller for a new run.
    pub async fn reset(&self) {
        let mut state = self.state.lock().await;
        *state = LoopState::default();

        let mut start = self.start_time.lock().await;
        *start = None;
    }

    /// Run the agent loop with the provided callbacks.
    ///
    /// # Arguments
    /// * `goal` - The task goal
    /// * `observe_fn` - Callback to get current browser state
    /// * `decide_fn` - Callback to get LLM decision
    /// * `execute_fn` - Callback to execute actions
    ///
    /// # Returns
    /// The final result of the loop execution.
    pub async fn run<O, D, E>(
        &self,
        goal: &str,
        mut observe_fn: O,
        mut decide_fn: D,
        mut execute_fn: E,
    ) -> AgentLoopResult
    where
        O: FnMut() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<BrowserStateSummary, String>> + Send>,
        >,
        D: FnMut(
            &BrowserStateSummary,
            &[AgentHistoryEntry],
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<AgentOutput, String>> + Send>,
        >,
        E: FnMut(
            &AgentAction,
            &BrowserStateSummary,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<AgentActionResult, String>> + Send>,
        >,
    {
        // Initialize
        {
            let mut state = self.state.lock().await;
            *state = LoopState::default();

            let mut start = self.start_time.lock().await;
            *start = Some(Instant::now());
        }

        loop {
            // Check termination conditions
            let (should_terminate, result) = self.check_termination().await;
            if should_terminate {
                return match result {
                    Some(r) => r,
                    None => {
                        let state_guard = self.state.lock().await;
                        let elapsed = self.elapsed_ms().await;
                        AgentLoopResult::failed(
                            "Unexpected termination".to_string(),
                            state_guard.step_count,
                            state_guard.history.clone(),
                            elapsed,
                        )
                    }
                };
            }

            // Increment step
            {
                let mut state = self.state.lock().await;
                state.step_count += 1;
            }

            let step_num = self.step_count().await;
            let history = self.history().await;

            // Execute one step
            match self
                .execute_step(
                    goal,
                    step_num,
                    &history,
                    &mut observe_fn,
                    &mut decide_fn,
                    &mut execute_fn,
                )
                .await
            {
                Ok(step_result) => {
                    let mut state = self.state.lock().await;
                    state.history.push(step_result.history_entry);
                    state.consecutive_failures = 0;

                    if step_result.is_done {
                        state.is_done = true;
                        if let Some(done) = step_result.done_result {
                            let elapsed = self.elapsed_ms().await;
                            state.final_result = Some(if done.success {
                                AgentLoopResult::completed(
                                    done.message,
                                    state.step_count,
                                    state.history.clone(),
                                    elapsed,
                                )
                            } else {
                                AgentLoopResult::failed(
                                    done.message,
                                    state.step_count,
                                    state.history.clone(),
                                    elapsed,
                                )
                            });
                        }
                    }
                }
                Err(err) => {
                    let mut state = self.state.lock().await;
                    state.consecutive_failures += 1;
                    state.history.push(AgentHistoryEntry::error(step_num, err));
                }
            }
        }
    }

    /// Execute a single step of the loop.
    async fn execute_step<O, D, E>(
        &self,
        _goal: &str,
        step_num: u32,
        history: &[AgentHistoryEntry],
        observe_fn: &mut O,
        decide_fn: &mut D,
        execute_fn: &mut E,
    ) -> Result<StepResult, String>
    where
        O: FnMut() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<BrowserStateSummary, String>> + Send>,
        >,
        D: FnMut(
            &BrowserStateSummary,
            &[AgentHistoryEntry],
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<AgentOutput, String>> + Send>,
        >,
        E: FnMut(
            &AgentAction,
            &BrowserStateSummary,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<AgentActionResult, String>> + Send>,
        >,
    {
        // 1. Observe: Get current browser state
        let browser_state = observe_fn().await?;

        // 2. Think: Ask LLM to decide next actions
        let agent_output = decide_fn(&browser_state, history).await?;

        // 3. Act: Execute actions
        let mut action_results = Vec::new();
        let mut is_done = false;
        let mut done_result = None;

        let max_actions = self.config.max_actions_per_step as usize;

        for (i, action) in agent_output.actions.iter().enumerate() {
            if i >= max_actions {
                break;
            }

            // Check for done action
            if matches!(action.action_type, AgentActionType::Done) {
                is_done = true;
                let success = action
                    .params
                    .done_success
                    .or(action.params.success)
                    .unwrap_or(false);
                let text = action
                    .params
                    .done_text
                    .clone()
                    .unwrap_or_else(|| "Task completed".to_string());
                done_result = Some(DoneResult {
                    success,
                    message: text,
                });
                break;
            }

            // Execute action
            let result = execute_fn(action, &browser_state).await?;
            action_results.push(result.clone());

            // Wait between actions
            if i < agent_output.actions.len() - 1 {
                tokio::time::sleep(Duration::from_millis(self.config.wait_between_actions_ms))
                    .await;
            }

            // Stop if action failed
            if !result.success {
                break;
            }
        }

        // Aggregate results
        let overall_result = AgentActionResult {
            success: action_results.iter().all(|r| r.success),
            error_message: action_results.iter().find_map(|r| r.error_message.clone()),
            state_changed: action_results.iter().any(|r| r.state_changed),
        };

        Ok(StepResult {
            history_entry: AgentHistoryEntry {
                step_number: step_num,
                state_summary: format!("URL: {}", browser_state.url),
                actions_taken: agent_output.actions.clone(),
                result: overall_result,
                thinking: Some(agent_output.thinking.clone()),
                next_goal: Some(agent_output.next_goal.clone()),
                evaluation: agent_output.evaluation_previous_goal.clone(),
                memory: agent_output.memory.clone(),
            },
            is_done,
            done_result,
        })
    }

    /// Check termination conditions.
    async fn check_termination(&self) -> (bool, Option<AgentLoopResult>) {
        let state = self.state.lock().await;
        let elapsed = self.elapsed_ms().await;

        // Already done
        if state.is_done {
            return (true, state.final_result.clone());
        }

        // Cancelled
        if state.is_cancelled {
            return (
                true,
                Some(AgentLoopResult {
                    status: AgentLoopStatus::Cancelled,
                    message: "Loop cancelled by user".to_string(),
                    steps_taken: state.step_count,
                    final_output: None,
                    history: state.history.clone(),
                    total_time_ms: elapsed,
                }),
            );
        }

        // Max steps reached
        if state.step_count >= self.config.max_steps {
            return (
                true,
                Some(AgentLoopResult::max_steps_reached(
                    state.step_count,
                    state.history.clone(),
                    elapsed,
                )),
            );
        }

        // Too many failures
        if state.consecutive_failures >= self.config.max_consecutive_failures {
            return (
                true,
                Some(AgentLoopResult::failed(
                    format!(
                        "Too many consecutive failures: {}",
                        state.consecutive_failures
                    ),
                    state.step_count,
                    state.history.clone(),
                    elapsed,
                )),
            );
        }

        (false, None)
    }

    /// Get elapsed time in milliseconds.
    async fn elapsed_ms(&self) -> u64 {
        let start = self.start_time.lock().await;
        start.map(|s| s.elapsed().as_millis() as u64).unwrap_or(0)
    }
}

/// Aggregate multiple action results into one.
pub fn aggregate_action_results(results: &[AgentActionResult]) -> AgentActionResult {
    AgentActionResult {
        success: results.iter().all(|r| r.success),
        error_message: results.iter().find_map(|r| r.error_message.clone()),
        state_changed: results.iter().any(|r| r.state_changed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_controller_creation() {
        let controller = AgentLoopController::default_config();
        assert_eq!(controller.config().max_steps, 100);
        assert_eq!(controller.step_count().await, 0);
    }

    #[tokio::test]
    async fn test_controller_cancel() {
        let controller = AgentLoopController::default_config();
        assert!(!controller.is_cancelled().await);
        controller.cancel().await;
        assert!(controller.is_cancelled().await);
    }

    #[tokio::test]
    async fn test_result_constructors() {
        let completed = AgentLoopResult::completed("Done".to_string(), 5, vec![], 1000);
        assert!(completed.is_success());
        assert_eq!(completed.status, AgentLoopStatus::Completed);

        let failed = AgentLoopResult::failed("Error".to_string(), 3, vec![], 500);
        assert!(!failed.is_success());
        assert_eq!(failed.status, AgentLoopStatus::Failed);

        let max_steps = AgentLoopResult::max_steps_reached(100, vec![], 5000);
        assert!(!max_steps.is_success());
        assert_eq!(max_steps.status, AgentLoopStatus::MaxStepsReached);
    }

    #[test]
    fn test_aggregate_results() {
        let results = vec![
            AgentActionResult {
                success: true,
                error_message: None,
                state_changed: true,
            },
            AgentActionResult {
                success: true,
                error_message: None,
                state_changed: false,
            },
        ];

        let aggregated = aggregate_action_results(&results);
        assert!(aggregated.success);
        assert!(aggregated.state_changed);
        assert!(aggregated.error_message.is_none());

        let results_with_failure = vec![
            AgentActionResult {
                success: true,
                error_message: None,
                state_changed: false,
            },
            AgentActionResult {
                success: false,
                error_message: Some("Failed".to_string()),
                state_changed: false,
            },
        ];

        let aggregated = aggregate_action_results(&results_with_failure);
        assert!(!aggregated.success);
        assert_eq!(aggregated.error_message, Some("Failed".to_string()));
    }
}
