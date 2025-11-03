//! Flow executor implementation

use crate::errors::FlowError;
use crate::strategies::{FailureHandler, FailureHandlerResult};
use crate::types::*;
use action_gate::{GateValidator, ValidationContext};
use action_primitives::{ActionPrimitives, ExecCtx, WaitTier};
use async_recursion::async_recursion;
use async_trait::async_trait;
use soulbrowser_core_types::ExecRoute;
use soulbrowser_policy_center::{default_snapshot, PolicyView};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};
use tokio::time::{timeout, Duration};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// Flow executor trait
#[async_trait]
pub trait FlowExecutor: Send + Sync {
    /// Execute a flow
    async fn execute(&self, flow: &Flow, route: &ExecRoute) -> Result<FlowResult, FlowError>;

    /// Validate flow structure
    fn validate_flow(&self, flow: &Flow) -> Result<(), FlowError>;
}

/// Default flow executor implementation
pub struct DefaultFlowExecutor {
    primitives: Arc<dyn ActionPrimitives>,
    gate_validator: Option<Arc<dyn GateValidator>>,
    failure_handler: Arc<dyn FailureHandler>,
    policy_view: Arc<PolicyView>,
}

impl DefaultFlowExecutor {
    /// Create a new flow executor
    pub fn new(
        primitives: Arc<dyn ActionPrimitives>,
        gate_validator: Option<Arc<dyn GateValidator>>,
        failure_handler: Arc<dyn FailureHandler>,
    ) -> Self {
        Self {
            primitives,
            gate_validator,
            failure_handler,
            policy_view: Arc::new(PolicyView::from(default_snapshot())),
        }
    }

    fn build_exec_ctx(&self, route: &ExecRoute) -> ExecCtx {
        let deadline = Instant::now() + StdDuration::from_secs(30);
        ExecCtx::new(
            ExecRoute::new(
                route.session.clone(),
                route.page.clone(),
                route.frame.clone(),
            ),
            deadline,
            CancellationToken::new(),
            (*self.policy_view).clone(),
        )
    }

    /// Execute a flow node
    #[async_recursion]
    async fn execute_node(
        &self,
        node: &FlowNode,
        route: &ExecRoute,
        context: &mut FlowContext,
        default_strategy: FailureStrategy,
    ) -> Result<Vec<StepResult>, FlowError> {
        match node {
            FlowNode::Sequence { steps } => {
                self.execute_sequence(steps, route, context, default_strategy)
                    .await
            }

            FlowNode::Parallel { steps, wait_all } => {
                self.execute_parallel(steps, route, context, default_strategy, *wait_all)
                    .await
            }

            FlowNode::Conditional {
                condition,
                then_branch,
                else_branch,
            } => {
                self.execute_conditional(
                    condition,
                    then_branch,
                    else_branch.as_deref(),
                    route,
                    context,
                    default_strategy,
                )
                .await
            }

            FlowNode::Loop {
                body,
                condition,
                max_iterations,
            } => {
                self.execute_loop(
                    body,
                    condition,
                    *max_iterations,
                    route,
                    context,
                    default_strategy,
                )
                .await
            }

            FlowNode::Action {
                id,
                action,
                expect,
                failure_strategy,
            } => {
                let strategy = failure_strategy.unwrap_or(default_strategy);
                let result = self
                    .execute_action(id, action, expect.as_ref(), route, context, strategy)
                    .await?;
                Ok(vec![result])
            }
        }
    }

    /// Execute sequence of steps
    async fn execute_sequence(
        &self,
        steps: &[FlowNode],
        route: &ExecRoute,
        context: &mut FlowContext,
        default_strategy: FailureStrategy,
    ) -> Result<Vec<StepResult>, FlowError> {
        debug!("Executing sequence with {} steps", steps.len());
        let mut results = Vec::new();

        for (i, step) in steps.iter().enumerate() {
            debug!("Executing sequence step {}/{}", i + 1, steps.len());
            let step_results = self
                .execute_node(step, route, context, default_strategy)
                .await?;

            // Update context with last step success
            if let Some(last) = step_results.last() {
                context.previous_step_success = last.success;
            }

            results.extend(step_results);
        }

        Ok(results)
    }

    /// Execute parallel steps
    async fn execute_parallel(
        &self,
        steps: &[FlowNode],
        route: &ExecRoute,
        context: &mut FlowContext,
        default_strategy: FailureStrategy,
        wait_all: bool,
    ) -> Result<Vec<StepResult>, FlowError> {
        debug!(
            "Executing {} steps in parallel (wait_all={})",
            steps.len(),
            wait_all
        );

        // For now, execute sequentially
        // TODO: Implement true parallel execution with proper lifetime management
        warn!("Parallel execution not yet implemented, falling back to sequential");

        let mut all_results = Vec::new();
        let mut any_success = false;

        for step in steps {
            let mut step_context = context.clone();
            match self
                .execute_node(step, route, &mut step_context, default_strategy)
                .await
            {
                Ok(step_results) => {
                    if step_results.iter().any(|r| r.success) {
                        any_success = true;
                    }
                    all_results.extend(step_results);
                }
                Err(e) => {
                    if wait_all {
                        return Err(FlowError::ParallelFailed(e.to_string()));
                    }
                }
            }
        }

        if !wait_all && !any_success {
            return Err(FlowError::ParallelFailed(
                "No parallel step succeeded".to_string(),
            ));
        }

        Ok(all_results)
    }

    /// Execute conditional branch
    async fn execute_conditional(
        &self,
        condition: &FlowCondition,
        then_branch: &FlowNode,
        else_branch: Option<&FlowNode>,
        route: &ExecRoute,
        context: &mut FlowContext,
        default_strategy: FailureStrategy,
    ) -> Result<Vec<StepResult>, FlowError> {
        debug!("Evaluating conditional");

        let condition_met = self.evaluate_condition(condition, route, context).await?;

        if condition_met {
            debug!("Condition met, executing then branch");
            self.execute_node(then_branch, route, context, default_strategy)
                .await
        } else if let Some(else_node) = else_branch {
            debug!("Condition not met, executing else branch");
            self.execute_node(else_node, route, context, default_strategy)
                .await
        } else {
            debug!("Condition not met, no else branch");
            Ok(Vec::new())
        }
    }

    /// Execute loop
    async fn execute_loop(
        &self,
        body: &FlowNode,
        condition: &LoopCondition,
        max_iterations: u32,
        route: &ExecRoute,
        context: &mut FlowContext,
        default_strategy: FailureStrategy,
    ) -> Result<Vec<StepResult>, FlowError> {
        debug!("Executing loop (max_iterations={})", max_iterations);
        let mut results = Vec::new();
        let mut iteration = 0;

        loop {
            // Check max iterations
            if iteration >= max_iterations {
                warn!("Loop exceeded max iterations: {}", max_iterations);
                return Err(FlowError::LoopExceeded(max_iterations));
            }

            // Update iteration count
            context.iteration_count = iteration;

            // Evaluate loop condition
            let should_continue = match condition {
                LoopCondition::While(cond) => self.evaluate_condition(cond, route, context).await?,
                LoopCondition::Until(cond) => {
                    !self.evaluate_condition(cond, route, context).await?
                }
                LoopCondition::Count(count) => iteration < *count,
                LoopCondition::Infinite => true,
            };

            if !should_continue {
                debug!(
                    "Loop condition not met, exiting after {} iterations",
                    iteration
                );
                break;
            }

            debug!("Executing loop iteration {}", iteration);

            // Execute loop body
            let iteration_results = self
                .execute_node(body, route, context, default_strategy)
                .await?;

            results.extend(iteration_results);
            iteration += 1;
        }

        Ok(results)
    }

    /// Execute single action
    async fn execute_action(
        &self,
        id: &str,
        action: &ActionType,
        expect: Option<&action_gate::ExpectSpec>,
        route: &ExecRoute,
        _context: &mut FlowContext,
        strategy: FailureStrategy,
    ) -> Result<StepResult, FlowError> {
        info!("Executing action: {}", id);
        let mut result = StepResult::new(id.to_string(), format!("{:?}", action));

        // Execute action with retry logic
        let mut attempt = 1;
        loop {
            match self.execute_action_once(action, route).await {
                Ok(report) => {
                    // Action succeeded
                    result = result.with_report(report.clone()).with_success();

                    // Validate post-conditions if specified
                    if let Some(expect_spec) = expect {
                        if let Some(validator) = &self.gate_validator {
                            let validation_context = ValidationContext::new();
                            match validator
                                .validate(expect_spec, &validation_context, route)
                                .await
                            {
                                Ok(gate_result) if gate_result.passed => {
                                    debug!("Post-conditions passed for action {}", id);
                                }
                                Ok(gate_result) => {
                                    let error = format!(
                                        "Post-conditions failed: {}",
                                        gate_result.reasons.join(", ")
                                    );
                                    warn!("{}", error);

                                    // Handle validation failure
                                    match self
                                        .failure_handler
                                        .handle_failure(
                                            id,
                                            strategy,
                                            FlowError::GateError(error.clone()),
                                            attempt,
                                        )
                                        .await
                                    {
                                        FailureHandlerResult::Retry { .. } => {
                                            result.retry_attempts = attempt;
                                            attempt += 1;
                                            continue;
                                        }
                                        FailureHandlerResult::Abort(msg) => {
                                            return Ok(result.with_error(msg).finish());
                                        }
                                        FailureHandlerResult::Continue(msg) => {
                                            return Ok(result.with_error(msg).finish());
                                        }
                                        FailureHandlerResult::UseFallback => {
                                            // TODO: Implement fallback
                                            return Ok(result.with_error(error).finish());
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("Post-condition validation error: {}", e);
                                }
                            }
                        }
                    }

                    return Ok(result.finish());
                }
                Err(e) => {
                    // Action failed
                    warn!("Action {} failed (attempt {}): {}", id, attempt, e);

                    // Handle failure
                    match self
                        .failure_handler
                        .handle_failure(id, strategy, e, attempt)
                        .await
                    {
                        FailureHandlerResult::Retry { .. } => {
                            result.retry_attempts = attempt;
                            attempt += 1;
                            continue;
                        }
                        FailureHandlerResult::Abort(msg) => {
                            return Ok(result.with_error(msg).finish());
                        }
                        FailureHandlerResult::Continue(msg) => {
                            return Ok(result.with_error(msg).finish());
                        }
                        FailureHandlerResult::UseFallback => {
                            // TODO: Implement fallback
                            return Ok(result
                                .with_error("Fallback not implemented".to_string())
                                .finish());
                        }
                    }
                }
            }
        }
    }

    /// Execute action once (without retry)
    async fn execute_action_once(
        &self,
        action: &ActionType,
        route: &ExecRoute,
    ) -> Result<action_primitives::ActionReport, FlowError> {
        let ctx = self.build_exec_ctx(route);

        match action {
            ActionType::Navigate { url, wait_tier } => {
                Ok(self.primitives.navigate(&ctx, url, *wait_tier).await?)
            }
            ActionType::Click { anchor, wait_tier } => {
                Ok(self.primitives.click(&ctx, anchor, *wait_tier).await?)
            }
            ActionType::TypeText {
                anchor,
                text,
                submit,
                wait_tier,
            } => Ok(self
                .primitives
                .type_text(&ctx, anchor, text, *submit, Some(*wait_tier))
                .await?),
            ActionType::Select {
                anchor,
                option,
                method,
                wait_tier,
            } => {
                let method = method.unwrap_or_default();
                let wait = wait_tier.unwrap_or(WaitTier::DomReady);
                Ok(self
                    .primitives
                    .select(&ctx, anchor, method, option, wait)
                    .await?)
            }
            ActionType::Scroll {
                target, behavior, ..
            } => Ok(self.primitives.scroll(&ctx, target, *behavior).await?),
            ActionType::Wait {
                condition,
                timeout_ms,
            } => Ok(self
                .primitives
                .wait_for(&ctx, condition, *timeout_ms)
                .await?),
            ActionType::Custom { action_type, .. } => Err(FlowError::ActionError(format!(
                "Custom action '{}' not supported",
                action_type
            ))),
        }
    }

    /// Evaluate flow condition
    #[async_recursion]
    async fn evaluate_condition(
        &self,
        condition: &FlowCondition,
        _route: &ExecRoute,
        context: &FlowContext,
    ) -> Result<bool, FlowError> {
        match condition {
            FlowCondition::PreviousStepSucceeded => Ok(context.previous_step_success),

            FlowCondition::VariableEquals { name, value } => {
                Ok(context.get_variable(name) == Some(value))
            }

            FlowCondition::And(conditions) => {
                for cond in conditions {
                    if !self.evaluate_condition(cond, _route, context).await? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }

            FlowCondition::Or(conditions) => {
                for cond in conditions {
                    if self.evaluate_condition(cond, _route, context).await? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }

            FlowCondition::Not(cond) => Ok(!self.evaluate_condition(cond, _route, context).await?),

            // TODO: Implement other conditions with CDP integration
            _ => {
                debug!("Condition evaluation not yet implemented, returning false");
                Ok(false)
            }
        }
    }
}

#[async_trait]
impl FlowExecutor for DefaultFlowExecutor {
    async fn execute(&self, flow: &Flow, route: &ExecRoute) -> Result<FlowResult, FlowError> {
        info!("Executing flow: {} ({})", flow.name, flow.id);

        // Validate flow structure
        self.validate_flow(flow)?;

        let mut result = FlowResult::new(flow.id.clone());
        let mut context = FlowContext::new();

        // Execute with timeout
        let flow_timeout = Duration::from_millis(flow.timeout_ms);

        match timeout(
            flow_timeout,
            self.execute_node(
                &flow.root,
                route,
                &mut context,
                flow.default_failure_strategy,
            ),
        )
        .await
        {
            Ok(Ok(step_results)) => {
                // Flow completed successfully
                let all_success = step_results.iter().all(|r| r.success);

                let mut finished = result.finish();
                for step_result in step_results {
                    finished = finished.with_step(step_result);
                }

                if all_success {
                    info!("Flow {} completed successfully", flow.id);
                    finished = finished.with_success();
                } else {
                    warn!("Flow {} completed with failures", flow.id);
                    finished = finished.with_error("Some steps failed".to_string());
                }

                // Copy variables from context
                finished.variables = context.variables;

                Ok(finished)
            }
            Ok(Err(e)) => {
                // Flow failed
                warn!("Flow {} failed: {}", flow.id, e);
                result = result.with_error(e.to_string()).finish();
                Ok(result)
            }
            Err(_) => {
                // Flow timed out
                warn!("Flow {} timed out after {}ms", flow.id, flow.timeout_ms);
                let _ = result
                    .with_error(format!("Flow timed out after {}ms", flow.timeout_ms))
                    .finish();
                Err(FlowError::Timeout(flow.timeout_ms))
            }
        }
    }

    fn validate_flow(&self, flow: &Flow) -> Result<(), FlowError> {
        debug!("Validating flow structure: {}", flow.id);

        // Basic validation
        if flow.id.is_empty() {
            return Err(FlowError::ValidationFailed(
                "Flow ID cannot be empty".to_string(),
            ));
        }

        if flow.timeout_ms == 0 {
            return Err(FlowError::ValidationFailed(
                "Flow timeout must be greater than 0".to_string(),
            ));
        }

        // Validate root node
        self.validate_node(&flow.root)?;

        debug!("Flow validation passed");
        Ok(())
    }
}

impl DefaultFlowExecutor {
    /// Validate flow node recursively
    fn validate_node(&self, node: &FlowNode) -> Result<(), FlowError> {
        match node {
            FlowNode::Sequence { steps } => {
                if steps.is_empty() {
                    return Err(FlowError::InvalidStructure(
                        "Sequence cannot be empty".to_string(),
                    ));
                }
                for step in steps {
                    self.validate_node(step)?;
                }
            }

            FlowNode::Parallel { steps, .. } => {
                if steps.is_empty() {
                    return Err(FlowError::InvalidStructure(
                        "Parallel cannot be empty".to_string(),
                    ));
                }
                for step in steps {
                    self.validate_node(step)?;
                }
            }

            FlowNode::Conditional {
                then_branch,
                else_branch,
                ..
            } => {
                self.validate_node(then_branch)?;
                if let Some(else_node) = else_branch {
                    self.validate_node(else_node)?;
                }
            }

            FlowNode::Loop {
                body,
                max_iterations,
                ..
            } => {
                if *max_iterations == 0 {
                    return Err(FlowError::InvalidStructure(
                        "Loop max_iterations must be greater than 0".to_string(),
                    ));
                }
                self.validate_node(body)?;
            }

            FlowNode::Action { id, .. } => {
                if id.is_empty() {
                    return Err(FlowError::InvalidStructure(
                        "Action ID cannot be empty".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategies::DefaultFailureHandler;

    // Mock ActionPrimitives for testing
    struct MockActionPrimitives;

    #[async_trait]
    impl ActionPrimitives for MockActionPrimitives {
        async fn navigate(
            &self,
            _ctx: &action_primitives::ExecCtx,
            _url: &str,
            _wait_tier: action_primitives::WaitTier,
        ) -> Result<action_primitives::ActionReport, action_primitives::ActionError> {
            unimplemented!()
        }

        async fn click(
            &self,
            _ctx: &action_primitives::ExecCtx,
            _anchor: &action_primitives::AnchorDescriptor,
            _wait_tier: action_primitives::WaitTier,
        ) -> Result<action_primitives::ActionReport, action_primitives::ActionError> {
            unimplemented!()
        }

        async fn type_text(
            &self,
            _ctx: &action_primitives::ExecCtx,
            _anchor: &action_primitives::AnchorDescriptor,
            _text: &str,
            _submit: bool,
            _wait_tier: Option<action_primitives::WaitTier>,
        ) -> Result<action_primitives::ActionReport, action_primitives::ActionError> {
            unimplemented!()
        }

        async fn select(
            &self,
            _ctx: &action_primitives::ExecCtx,
            _anchor: &action_primitives::AnchorDescriptor,
            _method: action_primitives::SelectMethod,
            _item: &str,
            _wait_tier: action_primitives::WaitTier,
        ) -> Result<action_primitives::ActionReport, action_primitives::ActionError> {
            unimplemented!()
        }

        async fn scroll(
            &self,
            _ctx: &action_primitives::ExecCtx,
            _target: &action_primitives::ScrollTarget,
            _behavior: action_primitives::ScrollBehavior,
        ) -> Result<action_primitives::ActionReport, action_primitives::ActionError> {
            unimplemented!()
        }

        async fn wait_for(
            &self,
            _ctx: &action_primitives::ExecCtx,
            _condition: &action_primitives::WaitCondition,
            _timeout_ms: u64,
        ) -> Result<action_primitives::ActionReport, action_primitives::ActionError> {
            unimplemented!()
        }
    }

    #[test]
    fn test_flow_validation_empty_id() {
        let executor = DefaultFlowExecutor::new(
            Arc::new(MockActionPrimitives),
            None,
            Arc::new(DefaultFailureHandler::new()),
        );

        let flow = Flow::new(
            "".to_string(),
            "test".to_string(),
            FlowNode::Sequence { steps: vec![] },
        );

        assert!(executor.validate_flow(&flow).is_err());
    }

    #[test]
    fn test_flow_validation_empty_sequence() {
        let executor = DefaultFlowExecutor::new(
            Arc::new(MockActionPrimitives),
            None,
            Arc::new(DefaultFailureHandler::new()),
        );

        let flow = Flow::new(
            "test_flow".to_string(),
            "test".to_string(),
            FlowNode::Sequence { steps: vec![] },
        );

        assert!(executor.validate_flow(&flow).is_err());
    }

    #[test]
    fn test_evaluate_condition_previous_step() {
        let executor = DefaultFlowExecutor::new(
            Arc::new(MockActionPrimitives),
            None,
            Arc::new(DefaultFailureHandler::new()),
        );

        let mut context = FlowContext::new();
        context.previous_step_success = true;

        let route = ExecRoute::new(
            soulbrowser_core_types::SessionId::new(),
            soulbrowser_core_types::PageId::new(),
            soulbrowser_core_types::FrameId::new(),
        );

        let result = tokio_test::block_on(executor.evaluate_condition(
            &FlowCondition::PreviousStepSucceeded,
            &route,
            &context,
        ));

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_evaluate_condition_variable_equals() {
        let executor = DefaultFlowExecutor::new(
            Arc::new(MockActionPrimitives),
            None,
            Arc::new(DefaultFailureHandler::new()),
        );

        let mut context = FlowContext::new();
        context.set_variable("count".to_string(), serde_json::json!(5));

        let route = ExecRoute::new(
            soulbrowser_core_types::SessionId::new(),
            soulbrowser_core_types::PageId::new(),
            soulbrowser_core_types::FrameId::new(),
        );

        let result = tokio_test::block_on(executor.evaluate_condition(
            &FlowCondition::VariableEquals {
                name: "count".to_string(),
                value: serde_json::json!(5),
            },
            &route,
            &context,
        ));

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_evaluate_condition_and() {
        let executor = DefaultFlowExecutor::new(
            Arc::new(MockActionPrimitives),
            None,
            Arc::new(DefaultFailureHandler::new()),
        );

        let mut context = FlowContext::new();
        context.previous_step_success = true;
        context.set_variable("count".to_string(), serde_json::json!(5));

        let route = ExecRoute::new(
            soulbrowser_core_types::SessionId::new(),
            soulbrowser_core_types::PageId::new(),
            soulbrowser_core_types::FrameId::new(),
        );

        let result = tokio_test::block_on(executor.evaluate_condition(
            &FlowCondition::And(vec![
                FlowCondition::PreviousStepSucceeded,
                FlowCondition::VariableEquals {
                    name: "count".to_string(),
                    value: serde_json::json!(5),
                },
            ]),
            &route,
            &context,
        ));

        assert!(result.is_ok());
        assert!(result.unwrap());
    }
}
