//! Failure handling strategies

use crate::errors::FlowError;
use crate::types::FailureStrategy;
use async_trait::async_trait;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

/// Failure handler trait
#[async_trait]
pub trait FailureHandler: Send + Sync {
    /// Handle step failure according to strategy
    async fn handle_failure(
        &self,
        step_id: &str,
        strategy: FailureStrategy,
        error: FlowError,
        attempt: u32,
    ) -> FailureHandlerResult;

    /// Check if retry should be attempted
    fn should_retry(&self, strategy: FailureStrategy, attempt: u32) -> bool;

    /// Calculate backoff duration for retry
    fn calculate_backoff(&self, strategy: FailureStrategy, attempt: u32) -> Duration;
}

/// Result of failure handling
#[derive(Debug, Clone)]
pub enum FailureHandlerResult {
    /// Abort the entire flow
    Abort(String),

    /// Continue to next step
    Continue(String),

    /// Retry the current step
    Retry { attempt: u32, backoff_ms: u64 },

    /// Use fallback node
    UseFallback,
}

/// Default failure handler implementation
pub struct DefaultFailureHandler;

impl DefaultFailureHandler {
    /// Create a new default failure handler
    pub fn new() -> Self {
        Self
    }
}

impl Default for DefaultFailureHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FailureHandler for DefaultFailureHandler {
    async fn handle_failure(
        &self,
        step_id: &str,
        strategy: FailureStrategy,
        error: FlowError,
        attempt: u32,
    ) -> FailureHandlerResult {
        match strategy {
            FailureStrategy::Abort => {
                warn!("Step {} failed, aborting flow: {}", step_id, error);
                FailureHandlerResult::Abort(error.to_string())
            }

            FailureStrategy::Continue => {
                warn!(
                    "Step {} failed, continuing to next step: {}",
                    step_id, error
                );
                FailureHandlerResult::Continue(error.to_string())
            }

            FailureStrategy::Retry {
                max_attempts,
                backoff_ms: _,
            } => {
                if attempt >= max_attempts {
                    warn!(
                        "Step {} failed after {} attempts, aborting: {}",
                        step_id, attempt, error
                    );
                    FailureHandlerResult::Abort(format!(
                        "Max retry attempts ({}) exceeded: {}",
                        max_attempts, error
                    ))
                } else {
                    let backoff = self.calculate_backoff(strategy, attempt);
                    info!(
                        "Step {} failed (attempt {}), retrying after {}ms",
                        step_id,
                        attempt,
                        backoff.as_millis()
                    );

                    // Apply backoff
                    sleep(backoff).await;

                    FailureHandlerResult::Retry {
                        attempt: attempt + 1,
                        backoff_ms: backoff.as_millis() as u64,
                    }
                }
            }

            FailureStrategy::Fallback => {
                info!("Step {} failed, using fallback: {}", step_id, error);
                FailureHandlerResult::UseFallback
            }
        }
    }

    fn should_retry(&self, strategy: FailureStrategy, attempt: u32) -> bool {
        match strategy {
            FailureStrategy::Retry { max_attempts, .. } => attempt < max_attempts,
            _ => false,
        }
    }

    fn calculate_backoff(&self, strategy: FailureStrategy, attempt: u32) -> Duration {
        match strategy {
            FailureStrategy::Retry { backoff_ms, .. } => {
                // Exponential backoff: backoff_ms * 2^(attempt-1)
                let multiplier = 2u64.pow(attempt.saturating_sub(1));
                let total_ms = backoff_ms.saturating_mul(multiplier);
                // Cap at 60 seconds
                let capped_ms = total_ms.min(60_000);
                Duration::from_millis(capped_ms)
            }
            _ => Duration::from_millis(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_retry() {
        let handler = DefaultFailureHandler::new();

        // Abort strategy - never retry
        assert!(!handler.should_retry(FailureStrategy::Abort, 0));
        assert!(!handler.should_retry(FailureStrategy::Abort, 5));

        // Continue strategy - never retry
        assert!(!handler.should_retry(FailureStrategy::Continue, 0));
        assert!(!handler.should_retry(FailureStrategy::Continue, 5));

        // Retry strategy - retry up to max_attempts
        let retry_strategy = FailureStrategy::Retry {
            max_attempts: 3,
            backoff_ms: 100,
        };
        assert!(handler.should_retry(retry_strategy, 0));
        assert!(handler.should_retry(retry_strategy, 1));
        assert!(handler.should_retry(retry_strategy, 2));
        assert!(!handler.should_retry(retry_strategy, 3));
        assert!(!handler.should_retry(retry_strategy, 4));
    }

    #[test]
    fn test_calculate_backoff() {
        let handler = DefaultFailureHandler::new();

        let strategy = FailureStrategy::Retry {
            max_attempts: 5,
            backoff_ms: 1000,
        };

        // Exponential backoff
        assert_eq!(handler.calculate_backoff(strategy, 1).as_millis(), 1000);
        assert_eq!(handler.calculate_backoff(strategy, 2).as_millis(), 2000);
        assert_eq!(handler.calculate_backoff(strategy, 3).as_millis(), 4000);
        assert_eq!(handler.calculate_backoff(strategy, 4).as_millis(), 8000);

        // Capped at 60 seconds
        assert_eq!(handler.calculate_backoff(strategy, 10).as_millis(), 60_000);
    }

    #[tokio::test]
    async fn test_handle_failure_abort() {
        let handler = DefaultFailureHandler::new();

        let result = handler
            .handle_failure(
                "test_step",
                FailureStrategy::Abort,
                FlowError::StepFailed {
                    step_id: "test".to_string(),
                    reason: "timeout".to_string(),
                },
                1,
            )
            .await;

        match result {
            FailureHandlerResult::Abort(msg) => {
                assert!(msg.contains("timeout"));
            }
            _ => panic!("Expected Abort result"),
        }
    }

    #[tokio::test]
    async fn test_handle_failure_continue() {
        let handler = DefaultFailureHandler::new();

        let result = handler
            .handle_failure(
                "test_step",
                FailureStrategy::Continue,
                FlowError::StepFailed {
                    step_id: "test".to_string(),
                    reason: "not found".to_string(),
                },
                1,
            )
            .await;

        match result {
            FailureHandlerResult::Continue(msg) => {
                assert!(msg.contains("not found"));
            }
            _ => panic!("Expected Continue result"),
        }
    }

    #[tokio::test]
    async fn test_handle_failure_retry() {
        let handler = DefaultFailureHandler::new();

        // First attempt - should retry
        let result = handler
            .handle_failure(
                "test_step",
                FailureStrategy::Retry {
                    max_attempts: 3,
                    backoff_ms: 100,
                },
                FlowError::StepFailed {
                    step_id: "test".to_string(),
                    reason: "transient error".to_string(),
                },
                1,
            )
            .await;

        match result {
            FailureHandlerResult::Retry { attempt, .. } => {
                assert_eq!(attempt, 2);
            }
            _ => panic!("Expected Retry result"),
        }

        // Max attempts reached - should abort
        let result = handler
            .handle_failure(
                "test_step",
                FailureStrategy::Retry {
                    max_attempts: 3,
                    backoff_ms: 100,
                },
                FlowError::StepFailed {
                    step_id: "test".to_string(),
                    reason: "transient error".to_string(),
                },
                3,
            )
            .await;

        match result {
            FailureHandlerResult::Abort(msg) => {
                assert!(msg.contains("Max retry attempts"));
            }
            _ => panic!("Expected Abort result"),
        }
    }

    #[tokio::test]
    async fn test_handle_failure_fallback() {
        let handler = DefaultFailureHandler::new();

        let result = handler
            .handle_failure(
                "test_step",
                FailureStrategy::Fallback,
                FlowError::StepFailed {
                    step_id: "test".to_string(),
                    reason: "error".to_string(),
                },
                1,
            )
            .await;

        match result {
            FailureHandlerResult::UseFallback => {}
            _ => panic!("Expected UseFallback result"),
        }
    }
}
