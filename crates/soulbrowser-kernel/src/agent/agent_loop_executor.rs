//! Agent Loop Executor for browser-use style iterative execution.
//!
//! This module provides the integration between the agent_loop module from agent-core
//! and the kernel's browser control capabilities.

use std::sync::Arc;
use std::time::Duration;

use agent_core::agent_loop::types::AgentActionResult;
use agent_core::{
    AgentAction, AgentActionType, AgentContext, AgentHistoryEntry as CoreHistoryEntry,
    AgentLocator, AgentLoopConfig, AgentOutput, AgentPlan, AgentPlanStep, AgentRequest,
    AgentScrollTarget, AgentTool, AgentToolKind, AgentWaitCondition, BrowserStateSummary,
    LlmProvider, PerceptionData, ScrollDirection, ScrollPosition, StateFormatter, WaitMode,
};
use anyhow::{anyhow, Context, Result};
use cdp_adapter::Cdp;
use perceiver_structural::{
    model::{Scope, SnapLevel},
    ports::AdapterPort,
    CdpPerceptionPort,
};
use serde::Serialize;
use soulbrowser_core_types::{ExecRoute, FrameId, RoutePrefer, RoutingHint};
use soulbrowser_registry::Registry;
use tracing::{debug, info, warn};

use crate::agent::{execute_plan, FlowExecutionOptions, StepExecutionStatus};
use crate::app_context::AppContext;

/// Options for agent loop execution.
#[derive(Clone, Debug)]
pub struct AgentLoopExecutionOptions {
    /// Maximum number of steps in the agent loop.
    pub max_steps: u32,
    /// Maximum actions per LLM decision.
    pub max_actions_per_step: u32,
    /// Enable vision (screenshot) for LLM.
    pub enable_vision: bool,
    /// Step timeout in milliseconds.
    pub step_timeout_ms: u64,
}

impl Default for AgentLoopExecutionOptions {
    fn default() -> Self {
        Self {
            max_steps: 50,
            max_actions_per_step: 3,
            enable_vision: false,
            step_timeout_ms: 30_000,
        }
    }
}

/// Result of agent loop execution.
#[derive(Debug, Serialize)]
pub struct AgentLoopExecutionReport {
    /// Whether the task completed successfully.
    pub success: bool,
    /// Final result text from the agent.
    pub result_text: Option<String>,
    /// Number of steps executed.
    pub steps_executed: u32,
    /// History of all steps.
    pub history: Vec<AgentLoopStepReport>,
    /// Error message if failed.
    pub error: Option<String>,
}

/// Report for a single step in the agent loop.
#[derive(Debug, Clone, Serialize)]
pub struct AgentLoopStepReport {
    /// Step number.
    pub step_number: u32,
    /// URL at this step.
    pub url: String,
    /// Actions taken.
    pub actions: Vec<AgentAction>,
    /// Aggregate execution result for the actions.
    pub result: AgentActionResult,
    /// LLM's thinking.
    pub thinking: Option<String>,
    /// LLM's next goal.
    pub next_goal: Option<String>,
    /// LLM's evaluation of the previous step.
    pub evaluation: Option<String>,
    /// LLM's memory for future steps.
    pub memory: Option<String>,
}

/// Execute an agent request in agent loop mode.
///
/// This function implements the observe-think-act loop:
/// 1. Observe: Get current browser state (DOM, accessibility tree, screenshot)
/// 2. Think: Send state to LLM and get next action(s)
/// 3. Act: Execute the action(s) in the browser
/// 4. Repeat until done or max steps reached
pub async fn execute_agent_loop(
    context: Arc<AppContext>,
    llm: Arc<dyn LlmProvider>,
    request: &AgentRequest,
    options: AgentLoopExecutionOptions,
) -> Result<AgentLoopExecutionReport> {
    info!(
        task_id = %request.task_id.0,
        goal = %request.goal,
        "Starting agent loop execution"
    );

    // Build agent loop config
    let config = AgentLoopConfig::new()
        .max_steps(options.max_steps)
        .actions_per_step(options.max_actions_per_step)
        .vision(options.enable_vision);

    // Get execution route from context/registry
    let route = resolve_exec_route(&context, request.context.as_ref()).await?;

    // Create a modified request with the resolved route's session/page in context
    // This ensures execute_plan uses the same page as observe_browser_state
    let request = {
        let mut modified_request = request.clone();
        let mut ctx = modified_request.context.take().unwrap_or_default();
        ctx.session = Some(route.session.clone());
        ctx.page = Some(route.page.clone());
        modified_request.context = Some(ctx);
        modified_request
    };
    let request = &request;

    // Create state formatter
    let state_formatter = StateFormatter::new(&config);

    // Track execution history
    let mut history: Vec<AgentLoopStepReport> = Vec::new();
    let mut step_count = 0u32;
    let mut final_result: Option<String> = None;
    let mut final_success = false;
    let mut final_error: Option<String> = None;

    loop {
        step_count += 1;

        if step_count > options.max_steps {
            warn!(task_id = %request.task_id.0, "Max steps reached");
            final_error = Some("Max steps reached without completion".to_string());
            break;
        }

        // 1. Observe - Get current browser state
        let state = match observe_browser_state(&context, &route, &state_formatter, Some(&request.task_id.0)).await {
            Ok(s) => s,
            Err(e) => {
                final_error = Some(format!("Observe failed: {}", e));
                break;
            }
        };

        // 2. Think - Call LLM to decide next action
        let core_history: Vec<CoreHistoryEntry> = history
            .iter()
            .map(|h| CoreHistoryEntry {
                step_number: h.step_number,
                state_summary: h.url.clone(),
                actions_taken: h.actions.clone(),
                result: h.result.clone(),
                thinking: h.thinking.clone(),
                next_goal: h.next_goal.clone(),
                evaluation: h.evaluation.clone(),
                memory: h.memory.clone(),
            })
            .collect();

        let output = match call_llm_decide(&llm, request, &state, &core_history).await {
            Ok(o) => o,
            Err(e) => {
                final_error = Some(format!("LLM decide failed: {}", e));
                break;
            }
        };

        // Prepare actions (respect max per step and stop if done is encountered)
        let max_actions = config.max_actions_per_step as usize;
        let mut actions_to_run: Vec<AgentAction> = Vec::new();
        let mut done_action: Option<AgentAction> = None;
        for (idx, action) in output.actions.iter().enumerate() {
            if idx >= max_actions {
                break;
            }
            if matches!(action.action_type, AgentActionType::Done) {
                done_action = Some(action.clone());
                break;
            }
            actions_to_run.push(action.clone());
        }

        // 3. Act - Execute the actions (if any)
        let exec_result: AgentActionResult = if actions_to_run.is_empty() {
            if let Some(done_action) = done_action.clone() {
                // Immediate completion without interactions
                history.push(AgentLoopStepReport {
                    step_number: step_count,
                    url: state.url.clone(),
                    actions: Vec::new(),
                    result: AgentActionResult {
                        success: true,
                        error_message: None,
                        state_changed: false,
                    },
                    thinking: Some(output.thinking.clone()),
                    next_goal: Some(output.next_goal.clone()),
                    evaluation: output.evaluation_previous_goal.clone(),
                    memory: output.memory.clone(),
                });
                let (success, text) = done_action_result(&done_action);
                final_success = success;
                final_result = Some(text);
                info!(task_id = %request.task_id.0, "Agent signaled done");
                break;
            } else {
                final_error = Some("LLM did not provide executable actions".to_string());
                break;
            }
        } else {
            match execute_actions(
                &context,
                request,
                &config,
                &state,
                &actions_to_run,
                step_count,
            )
            .await
            {
                Ok(result) => result,
                Err(e) => {
                    final_error = Some(format!("Action execution failed: {}", e));
                    break;
                }
            }
        };

        history.push(AgentLoopStepReport {
            step_number: step_count,
            url: state.url.clone(),
            actions: actions_to_run.clone(),
            result: exec_result.clone(),
            thinking: Some(output.thinking.clone()),
            next_goal: Some(output.next_goal.clone()),
            evaluation: output.evaluation_previous_goal.clone(),
            memory: output.memory.clone(),
        });

        if !exec_result.success {
            if let Some(err) = exec_result.error_message.as_deref() {
                warn!(task_id = %request.task_id.0, error = err, "Action execution failed");
            }
        }

        if let Some(done_action) = done_action {
            let (success, text) = done_action_result(&done_action);
            final_success = success;
            final_result = Some(text);
            info!(task_id = %request.task_id.0, "Agent signaled done");
            break;
        }

        // Brief pause between steps
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Ok(AgentLoopExecutionReport {
        success: final_success,
        result_text: final_result,
        steps_executed: step_count,
        history,
        error: final_error,
    })
}

/// Resolve an execution route from the current agent context and registry state.
///
/// This function ensures a browser session with a ready page exists before returning.
/// It follows the same pattern as Plan-Execute mode's `ensure_session_ready`.
async fn resolve_exec_route(
    context: &Arc<AppContext>,
    agent_ctx: Option<&AgentContext>,
) -> Result<ExecRoute> {
    let registry = context.registry();
    let mut hint = build_agent_loop_hint(agent_ctx);
    let mut sessions = registry.session_list().await;

    // Try to find a session with a focused page from the hint
    let mut target_ctx = hint
        .as_ref()
        .and_then(|h| h.session.as_ref())
        .and_then(|id| sessions.iter().find(|ctx| ctx.id == *id).cloned());

    // If the hinted session has a focused page, use it
    let has_usable_page = target_ctx
        .as_ref()
        .map(|ctx| ctx.focused_page.is_some())
        .unwrap_or(false);

    if !has_usable_page {
        // Try to find any session with a focused page
        target_ctx = sessions
            .iter()
            .find(|ctx| ctx.focused_page.is_some())
            .cloned();
    }

    // If still no session with page, create a new session
    let session_id = if let Some(ctx) = &target_ctx {
        ctx.id.clone()
    } else {
        let session = registry
            .session_create("agent-loop")
            .await
            .map_err(|err| anyhow!("failed to create session: {}", err))?;
        info!(session_id = %session.0, "Created new browser session for agent loop");
        session
    };

    // Ensure the session has a focused page (like Plan-Execute mode does)
    sessions = registry.session_list().await;
    target_ctx = sessions
        .iter()
        .find(|ctx| ctx.id == session_id)
        .cloned();

    let target_ctx = target_ctx.ok_or_else(|| anyhow!("session {} not found after creation", session_id.0))?;

    // If no focused page, open one
    let focused_page = if target_ctx.focused_page.is_some() {
        target_ctx.focused_page.clone()
    } else {
        info!(session_id = %session_id.0, "Opening new page for agent loop session");
        let page = registry
            .page_open(session_id.clone())
            .await
            .map_err(|err| anyhow!("failed to open page in session {}: {}", session_id.0, err))?;
        Some(page)
    };

    // Build routing hint with confirmed session and page
    hint = Some(RoutingHint {
        session: Some(session_id),
        page: focused_page,
        frame: hint.and_then(|h| h.frame),
        prefer: Some(RoutePrefer::Focused),
    });

    registry
        .route_resolve(hint)
        .await
        .map_err(|err| anyhow!("failed to resolve execution route: {}", err))
}

fn build_agent_loop_hint(context: Option<&AgentContext>) -> Option<RoutingHint> {
    let ctx = context?;
    let mut hint = RoutingHint::default();
    hint.session = ctx.session.clone();
    hint.page = ctx.page.clone();
    hint.frame = ctx
        .metadata
        .get("frame_id")
        .and_then(|value| value.as_str())
        .map(|frame| FrameId(frame.to_string()));
    hint.prefer = Some(RoutePrefer::Focused);
    Some(hint)
}

/// Observe the current browser state.
async fn observe_browser_state(
    context: &Arc<AppContext>,
    route: &ExecRoute,
    state_formatter: &StateFormatter,
    task_id: Option<&str>,
) -> Result<BrowserStateSummary> {
    // Get CdpAdapter from tool_manager
    let adapter = context
        .tool_manager()
        .cdp_adapter()
        .ok_or_else(|| anyhow!("CDP adapter not available - browser not connected"))?;

    // Ensure the CDP adapter is started (connects to Chrome).
    // This is normally done by the tool executor, but we need it for direct CDP operations.
    Arc::clone(&adapter)
        .start()
        .await
        .map_err(|e| anyhow!("Failed to start CDP adapter: {:?}", e))?;

    // Wait for the CDP adapter to have at least one page with a session.
    // The event loop needs time to process Target.attachedToTarget events.
    wait_for_cdp_session_ready(&adapter).await?;

    // Resolve the route to an actual CDP page.
    // This will create a new Chrome tab if needed (same as Plan-Execute mode).
    let resolved_ctx = adapter
        .resolve_execution_context(route)
        .await
        .map_err(|e| anyhow!("Failed to resolve execution context: {:?}", e))?;

    let adapter_page_id = resolved_ctx.page;
    debug!(
        route_page = %route.page.0,
        cdp_page = ?adapter_page_id,
        "Resolved route to CDP page"
    );

    // Wait for the page to be ready for DOM operations.
    // The CDP session might exist but the page's execution context may not be ready yet.
    // We verify readiness by attempting a simple script evaluation.
    let page_ready = wait_for_page_dom_ready(&adapter, adapter_page_id).await;
    if let Err(e) = page_ready {
        warn!(
            cdp_page = ?adapter_page_id,
            error = %e,
            "Page DOM not ready, proceeding anyway"
        );
    }

    // Create a modified route with the resolved CDP page ID.
    // The perception port's sample_dom_ax parses the page ID from the route,
    // so we need to use the CDP adapter's page ID for the DOM snapshot to work.
    let cdp_page_id = soulbrowser_core_types::PageId(adapter_page_id.0.to_string());
    let cdp_route = ExecRoute::new(
        route.session.clone(),
        cdp_page_id.clone(),
        route.frame.clone(),
    );

    // Create perception port for DOM/AX sampling
    let perception_port = Arc::new(AdapterPort::new(Arc::clone(&adapter)));

    // Determine scope from the CDP route
    let scope = Scope::Page(cdp_page_id);

    // Sample DOM and AX trees using the CDP-resolved route
    let sampled = perception_port
        .sample_dom_ax(&cdp_route, &scope, SnapLevel::Full)
        .await
        .map_err(|e| anyhow!("DOM/AX sampling failed: {:?}", e))?;

    // Get current URL via script evaluation for real-time accuracy
    // The registry may not be updated immediately after navigation
    let current_url = match adapter
        .evaluate_script(adapter_page_id, "window.location.href")
        .await
    {
        Ok(val) => val
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "about:blank".to_string()),
        Err(_) => adapter
            .registry()
            .get(&adapter_page_id)
            .and_then(|ctx| ctx.recent_url)
            .unwrap_or_else(|| "about:blank".to_string()),
    };

    // Get page title via script evaluation (optional - graceful failure)
    let title: Option<String> = match adapter
        .evaluate_script(adapter_page_id, "document.title")
        .await
    {
        Ok(val) => val.as_str().map(|s: &str| s.to_string()),
        Err(_) => None,
    };

    // Get scroll position via script evaluation (optional)
    let scroll_position: Option<ScrollPosition> = match adapter
        .evaluate_script(
            adapter_page_id,
            r#"JSON.stringify({
                scrollY: window.scrollY || window.pageYOffset || 0,
                scrollHeight: document.documentElement.scrollHeight || document.body.scrollHeight || 0,
                clientHeight: window.innerHeight || document.documentElement.clientHeight || 0
            })"#,
        )
        .await
    {
        Ok(val) => {
            if let Some(json_str) = val.as_str() {
                serde_json::from_str::<ScrollInfo>(json_str)
                    .ok()
                    .map(|info| ScrollPosition {
                        pixels_from_top: info.scroll_y,
                        total_height: info.scroll_height,
                        viewport_height: info.client_height,
                    })
            } else {
                None
            }
        }
        Err(_) => None,
    };

    // Get screenshot if vision is enabled
    let screenshot_base64: Option<String> = if state_formatter.is_vision_enabled() {
        match adapter
            .screenshot(adapter_page_id, Default::default())
            .await
        {
            Ok(screenshot_data) => {
                use base64::Engine;
                Some(base64::engine::general_purpose::STANDARD.encode(&screenshot_data))
            }
            Err(e) => {
                warn!(error = ?e, "Screenshot capture failed");
                None
            }
        }
    } else {
        None
    };

    // Emit screenshot as observation for real-time view in frontend
    if let (Some(screenshot), Some(tid)) = (&screenshot_base64, task_id) {
        use serde_json::json;
        use soulbrowser_core_types::TaskId;

        let artifact = json!({
            "content_type": "image/png",
            "data_base64": screenshot,
            "route": {
                "session": route.session.0,
                "page": route.page.0,
            },
            "label": "agent_loop_observation",
        });

        if let Some(handle) = context
            .task_status_registry()
            .handle(TaskId(tid.to_string()))
        {
            handle.push_evidence(&[artifact]);
            debug!(task_id = %tid, "Emitted screenshot observation for live view");
        }
    }

    // Build perception data
    let mut perception = PerceptionData::new(sampled.dom, sampled.ax, current_url);

    if let Some(t) = title {
        perception = perception.with_title(t);
    }

    if let Some(scroll) = scroll_position {
        perception = perception.with_scroll(scroll);
    }

    if let Some(screenshot) = screenshot_base64 {
        perception = perception.with_screenshot(screenshot);
    }

    debug!(
        url = %perception.url,
        has_title = perception.title.is_some(),
        has_screenshot = perception.screenshot_base64.is_some(),
        "Browser state observed"
    );

    Ok(state_formatter.format_from_perception(&perception))
}

/// Helper struct for parsing scroll info JSON.
#[derive(serde::Deserialize)]
struct ScrollInfo {
    #[serde(rename = "scrollY")]
    scroll_y: i32,
    #[serde(rename = "scrollHeight")]
    scroll_height: i32,
    #[serde(rename = "clientHeight")]
    client_height: i32,
}

/// Call the LLM to decide the next action.
async fn call_llm_decide(
    llm: &Arc<dyn LlmProvider>,
    request: &AgentRequest,
    state: &BrowserStateSummary,
    history: &[CoreHistoryEntry],
) -> Result<AgentOutput> {
    llm.decide(request, state, history)
        .await
        .map_err(|err| anyhow!("LLM decide failed: {}", err))
}

/// Execute actions in the browser by converting them into a transient plan.
async fn execute_actions(
    context: &Arc<AppContext>,
    request: &AgentRequest,
    config: &AgentLoopConfig,
    state: &BrowserStateSummary,
    actions: &[AgentAction],
    step_number: u32,
) -> Result<AgentActionResult> {
    if actions.is_empty() {
        return Ok(AgentActionResult {
            success: true,
            error_message: None,
            state_changed: false,
        });
    }

    let plan = actions_to_plan(request, state, config, actions, step_number)?;
    let report = execute_plan(
        Arc::clone(context),
        request,
        &plan,
        FlowExecutionOptions::default(),
        None,
    )
    .await?;

    let error = report
        .steps
        .iter()
        .find(|step| step.status == StepExecutionStatus::Failed)
        .and_then(|step| step.error.clone());

    Ok(AgentActionResult {
        success: report.success,
        error_message: error,
        state_changed: report.success,
    })
}

fn actions_to_plan(
    request: &AgentRequest,
    state: &BrowserStateSummary,
    config: &AgentLoopConfig,
    actions: &[AgentAction],
    step_number: u32,
) -> Result<AgentPlan> {
    let mut plan = AgentPlan::new(
        request.task_id.clone(),
        format!("Agent loop step {}", step_number),
    );
    plan.description = format!("Auto-generated from agent loop step {}", step_number);

    for (idx, action) in actions.iter().enumerate() {
        let tool_kind = convert_action_to_tool(action, state)
            .with_context(|| format!("failed to map action {:?} to a tool", action.action_type))?;
        let tool = AgentTool {
            kind: tool_kind,
            wait: WaitMode::DomReady,
            timeout_ms: Some(config.action_timeout_ms),
        };
        let mut step = AgentPlanStep::new(
            format!("agent-loop-{}-{}", step_number, idx + 1),
            format!("{:?}", action.action_type),
            tool,
        );
        step.detail = format!("Generated from agent loop action {:?}", action.action_type);
        plan.push_step(step);
    }

    Ok(plan)
}

fn convert_action_to_tool(
    action: &AgentAction,
    state: &BrowserStateSummary,
) -> Result<AgentToolKind> {
    let params = &action.params;
    match action.action_type {
        AgentActionType::Navigate => {
            let url = params
                .url
                .clone()
                .context("navigate action missing url field")?;
            Ok(AgentToolKind::Navigate { url })
        }
        AgentActionType::Click => {
            let locator = locator_from_index(state, action.element_index)?;
            Ok(AgentToolKind::Click { locator })
        }
        AgentActionType::TypeText => {
            let locator = locator_from_index(state, action.element_index)?;
            let text = params
                .text
                .clone()
                .context("type_text action missing text field")?;
            let submit = params.submit.unwrap_or(false);
            Ok(AgentToolKind::TypeText {
                locator,
                text,
                submit,
            })
        }
        AgentActionType::Select => {
            let locator = locator_from_index(state, action.element_index)?;
            let value = params
                .value
                .clone()
                .context("select action missing value field")?;
            Ok(AgentToolKind::Select {
                locator,
                value,
                method: None,
            })
        }
        AgentActionType::Scroll => {
            let amount = params.amount.unwrap_or(400).abs();
            let target = match params.direction.unwrap_or(ScrollDirection::Down) {
                ScrollDirection::Up => AgentScrollTarget::Pixels(-(amount as i32)),
                ScrollDirection::Down => AgentScrollTarget::Pixels(amount as i32),
                ScrollDirection::Left => AgentScrollTarget::Pixels(-(amount as i32)),
                ScrollDirection::Right => AgentScrollTarget::Pixels(amount as i32),
                ScrollDirection::ToElement => {
                    let locator = locator_from_index(state, action.element_index)?;
                    AgentScrollTarget::Selector(locator)
                }
            };
            Ok(AgentToolKind::Scroll { target })
        }
        AgentActionType::Wait => {
            let ms = params.ms.unwrap_or(1_000);
            Ok(AgentToolKind::Wait {
                condition: AgentWaitCondition::Duration(ms),
            })
        }
        AgentActionType::Done => Err(anyhow!("Done action should be handled by the loop")),
    }
}

fn locator_from_index(
    state: &BrowserStateSummary,
    element_index: Option<u32>,
) -> Result<AgentLocator> {
    let index = element_index.context("action missing element_index")?;
    let selector = state.selector_map.get(&index).context(format!(
        "no selector information for element index {}",
        index
    ))?;

    if let Some(css) = &selector.css_selector {
        return Ok(AgentLocator::Css(css.clone()));
    }
    if let Some(aria) = selector.aria_selector.as_ref() {
        return Ok(AgentLocator::Aria {
            role: aria.role.clone(),
            name: aria.name.clone(),
        });
    }
    if let Some(text) = &selector.text_content {
        if !text.is_empty() {
            return Ok(AgentLocator::Text {
                content: text.clone(),
                exact: false,
            });
        }
    }

    Err(anyhow!(
        "element index {} is missing usable locator information",
        index
    ))
}

fn done_action_result(action: &AgentAction) -> (bool, String) {
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
    (success, text)
}

/// Wait for the CDP adapter to have at least one page with an active session.
///
/// After calling adapter.start(), the event loop needs time to process
/// Target.attachedToTarget events. This function waits until at least
/// one page has a CDP session, which is required for DOM operations.
async fn wait_for_cdp_session_ready(adapter: &Arc<cdp_adapter::CdpAdapter>) -> Result<()> {
    const MAX_ATTEMPTS: u32 = 20;
    const DELAY_MS: u64 = 100;

    for attempt in 1..=MAX_ATTEMPTS {
        // Check if any page has a CDP session
        let has_session = adapter
            .registry()
            .iter()
            .into_iter()
            .any(|(_, ctx)| ctx.cdp_session.is_some());

        if has_session {
            debug!(attempt = attempt, "CDP session ready");
            return Ok(());
        }

        if attempt == MAX_ATTEMPTS {
            return Err(anyhow!(
                "No CDP session available after {} attempts ({} ms). \
                 Is Chrome running and connected?",
                MAX_ATTEMPTS,
                MAX_ATTEMPTS as u64 * DELAY_MS
            ));
        }

        debug!(
            attempt = attempt,
            "Waiting for CDP session to be ready"
        );
        tokio::time::sleep(Duration::from_millis(DELAY_MS)).await;
    }

    Ok(())
}

/// Wait for a page's DOM to be ready for operations.
///
/// This function attempts to verify the page is ready by executing a simple script.
/// It retries a few times with short delays if the page is not ready yet.
async fn wait_for_page_dom_ready(
    adapter: &Arc<cdp_adapter::CdpAdapter>,
    page_id: cdp_adapter::ids::PageId,
) -> Result<()> {
    const MAX_ATTEMPTS: u32 = 10;
    const DELAY_MS: u64 = 100;

    for attempt in 1..=MAX_ATTEMPTS {
        // Try to evaluate a simple script to verify the page is ready
        match adapter.evaluate_script(page_id, "document.readyState").await {
            Ok(result) => {
                let ready_state = result.as_str().unwrap_or("unknown");
                debug!(
                    cdp_page = ?page_id,
                    ready_state = ready_state,
                    attempt = attempt,
                    "Page ready state check"
                );
                // "loading", "interactive", or "complete"
                if ready_state == "complete" || ready_state == "interactive" {
                    return Ok(());
                }
                // Page exists but still loading - wait a bit
                tokio::time::sleep(Duration::from_millis(DELAY_MS)).await;
            }
            Err(e) => {
                if attempt == MAX_ATTEMPTS {
                    return Err(anyhow!(
                        "Page not ready after {} attempts: {:?}",
                        MAX_ATTEMPTS,
                        e
                    ));
                }
                debug!(
                    cdp_page = ?page_id,
                    attempt = attempt,
                    error = ?e,
                    "Page not ready yet, retrying"
                );
                tokio::time::sleep(Duration::from_millis(DELAY_MS)).await;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_options() {
        let opts = AgentLoopExecutionOptions::default();
        assert_eq!(opts.max_steps, 50);
        assert_eq!(opts.max_actions_per_step, 3);
        assert!(!opts.enable_vision);
    }
}
