use std::{env, fmt, path::Path, sync::Arc, time::Duration};

use agent_core::{AgentContext, AgentRequest, LlmProvider, MockLlmProvider};
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as Base64, Engine as _};
use chrono::Utc;
use serde_json::{json, Value};
use soulbrowser_core_types::TaskId;
use tokio::fs;
use tokio::time::timeout;
use tracing::{instrument, warn};

use crate::agent::executor::UserResult;
use crate::agent::{ChatRunner, ChatSessionOutput, FlowExecutionReport, StepExecutionStatus};
use crate::console_fixture::PerceptionExecResult;
use crate::llm::{
    anthropic::{ClaudeConfig, ClaudeLlmProvider},
    openai::{OpenAiConfig, OpenAiLlmProvider},
    LlmPlanCache,
};
use crate::perception_service::{PerceptionJob, PerceptionOutput};
use crate::plugin_registry::PluginRegistry;
use crate::server::ServeState;
use crate::task_status::{
    AgentHistoryEntry, AgentHistoryStatus, TaskStatusHandle, TaskStatusRegistry,
    TaskStatusSnapshot, TaskUserResult,
};
use crate::visualization::execution_artifacts_from_report;
use soulbrowser_state_center::StateEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlannerSelection {
    Rule,
    Llm,
}

impl PlannerSelection {
    pub fn from_str_case(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "rule" => Some(Self::Rule),
            "llm" => Some(Self::Llm),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            PlannerSelection::Rule => "rule",
            PlannerSelection::Llm => "llm",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LlmProviderSelection {
    OpenAi,
    Anthropic,
    Mock,
}

impl Default for LlmProviderSelection {
    fn default() -> Self {
        Self::OpenAi
    }
}

impl LlmProviderSelection {
    pub fn from_str_case(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "openai" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            "mock" => Some(Self::Mock),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            LlmProviderSelection::OpenAi => "openai",
            LlmProviderSelection::Anthropic => "anthropic",
            LlmProviderSelection::Mock => "mock",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LlmProviderConfig {
    pub model: Option<String>,
    pub api_base: Option<String>,
    pub temperature: Option<f32>,
    pub api_key: Option<String>,
    pub max_output_tokens: Option<u32>,
}

const OPENAI_KEY_ENV_VARS: &[&str] = &["SOULBROWSER_OPENAI_API_KEY", "OPENAI_API_KEY"];
const ANTHROPIC_KEY_ENV_VARS: &[&str] = &[
    "SOULBROWSER_CLAUDE_API_KEY",
    "CLAUDE_API_KEY",
    "ANTHROPIC_API_KEY",
];

#[derive(Clone)]
pub struct ChatRunnerBuild {
    pub runner: ChatRunner,
    pub planner_used: PlannerSelection,
    pub provider_used: Option<LlmProviderSelection>,
    pub fallback_reason: Option<String>,
}

pub fn build_chat_runner(
    planner: PlannerSelection,
    provider: Option<LlmProviderSelection>,
    config: LlmProviderConfig,
    cache: Option<Arc<LlmPlanCache>>,
    _registry: Option<Arc<PluginRegistry>>,
) -> Result<ChatRunnerBuild> {
    match planner {
        PlannerSelection::Rule => Ok(ChatRunnerBuild {
            runner: ChatRunner::default(),
            planner_used: PlannerSelection::Rule,
            provider_used: None,
            fallback_reason: None,
        }),
        PlannerSelection::Llm => {
            if let Some(choice) = resolve_provider_selection(provider) {
                match build_llm_provider(choice, &config) {
                    Ok(llm) => Ok(ChatRunnerBuild {
                        runner: ChatRunner::default().with_llm_backend(llm, cache),
                        planner_used: PlannerSelection::Llm,
                        provider_used: Some(choice),
                        fallback_reason: None,
                    }),
                    Err(err) => {
                        if choice != LlmProviderSelection::Mock
                            && err.downcast_ref::<MissingApiKeyError>().is_some()
                        {
                            warn!(
                                provider = ?choice,
                                "LLM provider missing API key; falling back to rule-based planner"
                            );
                            Ok(ChatRunnerBuild {
                                runner: ChatRunner::default(),
                                planner_used: PlannerSelection::Rule,
                                provider_used: None,
                                fallback_reason: Some(format!("LLM planner disabled: {}", err)),
                            })
                        } else {
                            Err(err)
                        }
                    }
                }
            } else {
                warn!(
                    "LLM planner requested but no provider configured; falling back to rule-based planner"
                );
                Ok(ChatRunnerBuild {
                    runner: ChatRunner::default(),
                    planner_used: PlannerSelection::Rule,
                    provider_used: None,
                    fallback_reason: Some(
                        "LLM planner disabled: no provider configured".to_string(),
                    ),
                })
            }
        }
    }
}

pub fn llm_cache_for_request(
    state: &ServeState,
    provider: Option<LlmProviderSelection>,
    model: Option<&str>,
) -> Option<Arc<LlmPlanCache>> {
    let pool = state.llm_cache.as_ref()?;
    let provider_label = provider.unwrap_or_default().label();
    let model_label = model.unwrap_or("default");
    match pool.scoped(&[state.tenant_id(), provider_label, model_label]) {
        Ok(cache) => Some(cache),
        Err(err) => {
            warn!(?err, "failed to build llm plan cache namespace");
            None
        }
    }
}

pub fn plan_payload(session: &ChatSessionOutput) -> Value {
    json!({
        "plan": session.plan,
        "flow": {
            "metadata": {
                "step_count": session.flow.step_count,
                "validation_count": session.flow.validation_count,
            },
            "definition": session.flow.flow,
        },
        "explanations": session.explanations,
    })
}

fn build_llm_provider(
    selection: LlmProviderSelection,
    config: &LlmProviderConfig,
) -> Result<Arc<dyn LlmProvider>> {
    match selection {
        LlmProviderSelection::OpenAi => build_openai_provider(config),
        LlmProviderSelection::Anthropic => build_anthropic_provider(config),
        LlmProviderSelection::Mock => Ok(Arc::new(MockLlmProvider::default())),
    }
}

fn resolve_provider_selection(
    requested: Option<LlmProviderSelection>,
) -> Option<LlmProviderSelection> {
    match requested {
        Some(selection) => Some(selection),
        None => {
            if env_secret_available(OPENAI_KEY_ENV_VARS) {
                Some(LlmProviderSelection::OpenAi)
            } else if env_secret_available(ANTHROPIC_KEY_ENV_VARS) {
                Some(LlmProviderSelection::Anthropic)
            } else {
                None
            }
        }
    }
}

fn build_openai_provider(config: &LlmProviderConfig) -> Result<Arc<dyn LlmProvider>> {
    let api_key = resolve_api_key(&config.api_key, OPENAI_KEY_ENV_VARS)?;
    let model = config
        .model
        .clone()
        .unwrap_or_else(|| "gpt-4o-mini".to_string());
    let api_base = config
        .api_base
        .clone()
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let temperature = config.temperature.unwrap_or(0.2);
    let provider = OpenAiLlmProvider::new(OpenAiConfig {
        api_key,
        model,
        api_base,
        temperature,
        timeout: Duration::from_secs(60),
    })
    .map_err(|err| anyhow!("failed to configure OpenAI provider: {err}"))?;
    Ok(Arc::new(provider))
}

fn build_anthropic_provider(config: &LlmProviderConfig) -> Result<Arc<dyn LlmProvider>> {
    let api_key = resolve_api_key(&config.api_key, ANTHROPIC_KEY_ENV_VARS)?;
    let model = config
        .model
        .clone()
        .unwrap_or_else(|| "claude-3-5-sonnet-20240620".to_string());
    let api_base = config
        .api_base
        .clone()
        .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());
    let temperature = config.temperature.unwrap_or(0.2);
    let max_tokens = config.max_output_tokens.unwrap_or(2_000);
    let provider = ClaudeLlmProvider::new(ClaudeConfig {
        api_key,
        model,
        api_base,
        temperature,
        max_tokens,
        timeout: Duration::from_secs(60),
    })
    .map_err(|err| anyhow!("failed to configure Anthropic provider: {err}"))?;
    Ok(Arc::new(provider))
}

fn resolve_api_key(
    source: &Option<String>,
    env_keys: &'static [&'static str],
) -> Result<String, MissingApiKeyError> {
    if let Some(value) = source.as_ref().and_then(|raw| sanitize_secret(raw)) {
        return Ok(value);
    }
    for key in env_keys {
        if let Some(value) = env::var(key).ok().and_then(|raw| sanitize_secret(&raw)) {
            return Ok(value);
        }
    }
    Err(MissingApiKeyError::new(env_keys))
}

fn env_secret_available(env_keys: &[&str]) -> bool {
    env_keys.iter().any(|key| {
        env::var(key)
            .ok()
            .and_then(|value| sanitize_secret(&value))
            .is_some()
    })
}

#[derive(Debug)]
struct MissingApiKeyError {
    env_keys: &'static [&'static str],
}

impl MissingApiKeyError {
    fn new(env_keys: &'static [&'static str]) -> Self {
        Self { env_keys }
    }
}

impl fmt::Display for MissingApiKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let joined = self.env_keys.join(" or ");
        write!(f, "missing API key; supply llm_api_key or set {}", joined)
    }
}

impl std::error::Error for MissingApiKeyError {}

fn sanitize_secret(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let unquoted = trimmed.trim_matches(|c| c == '"' || c == '\'');
    if unquoted.is_empty() {
        return None;
    }
    if !unquoted.chars().any(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let lowered = unquoted.to_ascii_lowercase();
    if matches!(
        lowered.as_str(),
        "your-api-key" | "replace-me" | "changeme" | "set-me" | "todo"
    ) {
        return None;
    }
    Some(unquoted.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env_vars<F: FnOnce() -> T, T>(vars: &[(&str, Option<&str>)], f: F) -> T {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        let mut previous = Vec::new();
        for (key, value) in vars {
            previous.push((key.to_string(), env::var(key).ok()));
            match value {
                Some(val) => env::set_var(key, val),
                None => env::remove_var(key),
            }
        }
        let result = f();
        for (key, value) in previous {
            match value {
                Some(val) => env::set_var(&key, val),
                None => env::remove_var(&key),
            }
        }
        result
    }

    #[test]
    fn sanitize_secret_filters_placeholder_values() {
        assert!(sanitize_secret("…").is_none());
        assert!(sanitize_secret("   ...  ").is_none());
        assert!(sanitize_secret("your-api-key").is_none());
        assert_eq!(
            sanitize_secret("  sk-test-123  "),
            Some("sk-test-123".to_string())
        );
    }

    #[test]
    fn provider_default_ignores_placeholder_env_keys() {
        with_env_vars(
            &[
                ("SOULBROWSER_OPENAI_API_KEY", Some("…")),
                ("SOULBROWSER_CLAUDE_API_KEY", None),
            ],
            || {
                assert_eq!(resolve_provider_selection(None), None);
            },
        );
    }

    #[test]
    fn explicit_mock_selection_wins_over_env_keys() {
        with_env_vars(
            &[("SOULBROWSER_OPENAI_API_KEY", Some("sk-example-key"))],
            || {
                assert_eq!(
                    resolve_provider_selection(Some(LlmProviderSelection::Mock)),
                    Some(LlmProviderSelection::Mock)
                );
            },
        );
    }

    #[test]
    fn build_chat_runner_falls_back_to_mock_without_keys() {
        with_env_vars(
            &[
                ("SOULBROWSER_OPENAI_API_KEY", None),
                ("SOULBROWSER_CLAUDE_API_KEY", None),
            ],
            || {
                let build = build_chat_runner(
                    PlannerSelection::Llm,
                    Some(LlmProviderSelection::OpenAi),
                    LlmProviderConfig::default(),
                    None,
                    None,
                )
                .expect("LLM runner should fall back when API keys missing");
                assert_eq!(build.planner_used, PlannerSelection::Rule);
                assert!(build.fallback_reason.is_some());
                // Ensure we can still build a request from the runner to avoid unused warnings.
                let _request = build
                    .runner
                    .request_from_prompt("test".into(), None, Vec::new());
            },
        );
    }

    #[test]
    fn provider_detection_honors_generic_openai_env() {
        with_env_vars(
            &[
                ("SOULBROWSER_OPENAI_API_KEY", None),
                ("OPENAI_API_KEY", Some("sk-generic")),
            ],
            || {
                assert_eq!(
                    resolve_provider_selection(None),
                    Some(LlmProviderSelection::OpenAi)
                );
            },
        );
    }
}

pub fn apply_blocker_hints(request: &mut AgentRequest, blocker: Option<&str>) {
    if let Some(kind) = blocker {
        request
            .metadata
            .insert("blocker_kind".to_string(), Value::String(kind.to_string()));
    }
}

pub fn propagate_browser_state_metadata(request: &mut AgentRequest, snapshot: Option<&Value>) {
    if let Some(state) = snapshot {
        request
            .metadata
            .insert("browser_context".to_string(), state.clone());
    }
}

pub fn apply_perception_metadata(context: &mut AgentContext, snapshot: &PerceptionExecResult) {
    if let Some(perception) = &snapshot.perception {
        if let Ok(value) = serde_json::to_value(perception) {
            context.metadata.insert("perception".to_string(), value);
        }
    }
    if let Some(image) = &snapshot.screenshot_base64 {
        context.metadata.insert(
            "context_screenshot_base64".to_string(),
            Value::String(image.clone()),
        );
    }
}

#[instrument(
    name = "soul.chat.context_capture",
    skip(state),
    fields(url = %url)
)]
pub async fn capture_chat_context_snapshot(
    state: &ServeState,
    url: &str,
    screenshot: bool,
    timeout_secs: Option<u64>,
) -> Result<PerceptionExecResult> {
    let semaphore = state.chat_context_semaphore.clone();
    let permit_fut = semaphore.acquire_owned();
    let _permit = if let Some(wait) = state.chat_context_wait {
        match timeout(wait, permit_fut).await {
            Ok(result) => result.context("chat context semaphore closed")?,
            Err(_) => {
                warn!(
                    wait_ms = wait.as_millis() as u64,
                    "chat context capture throttled due to concurrency limit"
                );
                return Err(anyhow!(
                    "context capture concurrency limit reached (waited {:?})",
                    wait
                ));
            }
        }
    } else {
        permit_fut.await.context("chat context semaphore closed")?
    };

    let timeout_limit = timeout_secs.unwrap_or(30);
    let job = PerceptionJob {
        url: url.to_string(),
        enable_structural: true,
        enable_visual: true,
        enable_semantic: true,
        enable_insights: true,
        capture_screenshot: screenshot,
        timeout_secs: timeout_limit,
        chrome_path: None,
        ws_url: state.ws_url.clone(),
        headful: false,
        viewport: None,
        cookies: Vec::new(),
        inject_script: None,
        allow_pooling: true,
    };

    let service = state.perception_service();
    let fut = service.perceive(job);
    let output = timeout(Duration::from_secs(timeout_limit), fut)
        .await
        .map_err(|_| anyhow!("context capture timed out"))??;
    Ok(perception_exec_from_output(output))
}

fn perception_exec_from_output(output: PerceptionOutput) -> PerceptionExecResult {
    let screenshot_base64 = output
        .screenshot
        .as_deref()
        .map(|bytes| Base64.encode(bytes));
    PerceptionExecResult {
        success: true,
        perception: Some(output.perception),
        screenshot_base64,
        stdout: output.log_lines.join("\n"),
        stderr: String::new(),
        error_message: None,
    }
}

pub fn perception_snapshot_value(snapshot: &PerceptionExecResult) -> Value {
    json!({
        "success": snapshot.success,
        "stdout": snapshot.stdout,
        "stderr": snapshot.stderr,
        "error": snapshot.error_message,
        "has_perception": snapshot.perception.is_some(),
        "screenshot_base64": snapshot.screenshot_base64,
    })
}

pub fn latest_observation_summary(
    registry: &Arc<TaskStatusRegistry>,
    task_id: &TaskId,
) -> Option<String> {
    let snapshot = registry.snapshot(&task_id.0)?;
    snapshot
        .observation_history
        .last()
        .and_then(|value| {
            value
                .get("summary")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
        })
        .or_else(|| {
            snapshot
                .recent_evidence
                .last()
                .map(|value| value.to_string())
        })
}

pub fn latest_obstruction_kind(
    registry: &Arc<TaskStatusRegistry>,
    task_id: &TaskId,
) -> Option<String> {
    let snapshot = registry.snapshot(&task_id.0)?;
    snapshot
        .alerts
        .last()
        .and_then(|alert| alert.kind.clone())
        .or_else(|| {
            snapshot
                .watchdog_events
                .last()
                .map(|event| event.kind.clone())
        })
}

pub fn agent_history_prompt(
    registry: &Arc<TaskStatusRegistry>,
    task_id: &TaskId,
    max_entries: usize,
) -> Option<String> {
    let snapshot = registry.snapshot(&task_id.0)?;
    if snapshot.agent_history.is_empty() || max_entries == 0 {
        return None;
    }
    let mut lines = Vec::new();
    for entry in snapshot
        .agent_history
        .iter()
        .rev()
        .take(max_entries)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let status = match entry.status {
            crate::task_status::AgentHistoryStatus::Success => "success",
            crate::task_status::AgentHistoryStatus::Failed => "failed",
        };
        lines.push(format!(
            "Step {} ({}): {}",
            entry.step_index + 1,
            status,
            entry.title
        ));
        if let Some(message) = entry.message.as_deref() {
            lines.push(format!("  note: {}", message));
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

pub fn build_telemetry_payload(snapshot: &TaskStatusSnapshot) -> Option<Value> {
    Some(json!({
        "task_id": snapshot.task_id,
        "status": snapshot.status,
        "total_steps": snapshot.total_steps,
        "current_step": snapshot.current_step,
        "current_step_title": snapshot.current_step_title,
        "started_at": snapshot.started_at,
        "finished_at": snapshot.finished_at,
        "last_error": snapshot.last_error,
        "last_updated_at": snapshot.last_updated_at,
    }))
}

pub fn sync_agent_execution_history(handle: &TaskStatusHandle, report: &FlowExecutionReport) {
    for (index, step) in report.steps.iter().enumerate() {
        handle.step_started(index, &step.title);
        let status = match step.status {
            StepExecutionStatus::Success => AgentHistoryStatus::Success,
            StepExecutionStatus::Failed => AgentHistoryStatus::Failed,
        };
        let timeline_entry = AgentHistoryEntry {
            timestamp: Utc::now(),
            step_index: index,
            step_id: step.step_id.clone(),
            title: step.title.clone(),
            status,
            attempts: step.attempts,
            message: step.error.clone(),
            observation_summary: None,
            obstruction: None,
            structured_summary: None,
            wait_ms: Some(step.total_wait_ms),
            run_ms: Some(step.total_run_ms),
            tool_kind: Some(step.tool_kind.clone()),
        };
        handle.push_agent_history(timeline_entry);
        match step.status {
            StepExecutionStatus::Success => handle.step_completed(index, &step.title),
            StepExecutionStatus::Failed => handle.step_failed(
                index,
                &step.title,
                step.error.as_deref().unwrap_or("execution failed"),
            ),
        }
    }

    let evidence = execution_artifacts_from_report(report);
    if !evidence.is_empty() {
        handle.push_evidence(&evidence);
    }

    if report.success {
        handle.mark_success();
    } else {
        let failure_reason = report
            .steps
            .iter()
            .rev()
            .find(|step| matches!(step.status, StepExecutionStatus::Failed))
            .and_then(|step| step.error.clone());
        handle.mark_failure(failure_reason);
    }

    let converted: Vec<TaskUserResult> = report
        .user_results
        .iter()
        .map(convert_user_result)
        .collect();
    handle.set_user_results(converted, report.missing_user_result);
}

pub fn flow_execution_report_payload(report: &FlowExecutionReport) -> Value {
    json!({
        "success": report.success,
        "steps": report.steps.iter().map(|step| {
            json!({
                "step_id": step.step_id,
                "title": step.title,
                "tool_kind": step.tool_kind,
                "status": match step.status {
                    StepExecutionStatus::Success => "success",
                    StepExecutionStatus::Failed => "failed",
                },
                "attempts": step.attempts,
                "error": step.error,
                "total_wait_ms": step.total_wait_ms,
                "total_run_ms": step.total_run_ms,
                "dispatches": step.dispatches.iter().map(|dispatch| {
                    json!({
                        "label": dispatch.label,
                        "action_id": dispatch.action_id,
                        "wait_ms": dispatch.wait_ms,
                        "run_ms": dispatch.run_ms,
                        "route": {
                            "session": dispatch.route.session.0,
                            "page": dispatch.route.page.0,
                            "frame": dispatch.route.frame.0,
                        },
                        "output": dispatch.output,
                        "error": dispatch.error,
                    })
                }).collect::<Vec<_>>()
            })
        }).collect::<Vec<_>>(),
        "user_results": report.user_results.iter().map(|result| {
            json!({
                "step_id": result.step_id,
                "step_title": result.step_title,
                "kind": result.kind.as_str(),
                "schema": result.schema,
                "content": result.content,
                "artifact_path": result.artifact_path,
            })
        }).collect::<Vec<_>>(),
        "missing_user_result": report.missing_user_result,
    })
}

fn convert_user_result(result: &UserResult) -> TaskUserResult {
    TaskUserResult {
        step_id: result.step_id.clone(),
        step_title: result.step_title.clone(),
        kind: result.kind.as_str().to_string(),
        schema: result.schema.clone(),
        content: if result.content.is_null() {
            None
        } else {
            Some(result.content.clone())
        },
        artifact_path: result.artifact_path.clone(),
    }
}

#[instrument(
    name = "soul.chat.persist_outputs",
    skip(execution_reports, state_events, telemetry_payload),
    fields(task_id = %task_id.0)
)]
pub async fn persist_execution_outputs(
    output_dir: &Path,
    task_id: &TaskId,
    plan_history: &[Value],
    execution_reports: &[FlowExecutionReport],
    state_events: Option<Vec<StateEvent>>,
    telemetry_payload: Option<Value>,
) -> Result<Vec<Value>> {
    let task_dir = output_dir.join("tasks").join(&task_id.0);
    fs::create_dir_all(&task_dir)
        .await
        .with_context(|| format!("failed to create task directory {}", task_dir.display()))?;

    let plans_path = task_dir.join("plans.json");
    fs::write(&plans_path, serde_json::to_vec_pretty(plan_history)?)
        .await
        .with_context(|| format!("failed to write plans {}", plans_path.display()))?;

    let execution_values: Vec<Value> = execution_reports
        .iter()
        .map(flow_execution_report_payload)
        .collect();
    let exec_path = task_dir.join("executions.json");
    fs::write(&exec_path, serde_json::to_vec_pretty(&execution_values)?)
        .await
        .with_context(|| format!("failed to write executions {}", exec_path.display()))?;

    if let Some(events) = state_events {
        let json_events: Vec<Value> = events
            .into_iter()
            .map(|event| Value::String(format!("{:?}", event)))
            .collect();
        let events_path = task_dir.join("state_events.json");
        fs::write(&events_path, serde_json::to_vec_pretty(&json_events)?)
            .await
            .with_context(|| format!("failed to write state events {}", events_path.display()))?;
    }

    if let Some(payload) = telemetry_payload {
        let telemetry_path = task_dir.join("telemetry.json");
        fs::write(&telemetry_path, serde_json::to_vec_pretty(&payload)?)
            .await
            .with_context(|| format!("failed to write telemetry {}", telemetry_path.display()))?;
    }

    let mut artifacts = Vec::new();
    for report in execution_reports {
        artifacts.extend(execution_artifacts_from_report(report));
    }
    Ok(artifacts)
}
