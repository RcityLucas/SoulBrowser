use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use serde_json::{json, Value};
use serde_yaml;
use tokio::fs;
use tracing::info;

use agent_core::{AgentContext, AgentRequest, ConversationRole, ConversationTurn};
use soulbrowser_kernel::agent::{
    execute_plan, ChatSessionOutput, FlowExecutionOptions, FlowExecutionReport, StepExecutionStatus,
};
use soulbrowser_kernel::chat_support::{
    build_chat_runner, ChatRunnerBuild, LlmProviderConfig, LlmProviderSelection, PlannerSelection,
};
use soulbrowser_kernel::plan_payload;
use soulbrowser_state_center::{DispatchStatus, PerceiverEventKind, StateEvent};

use crate::cli::context::CliContext;
use crate::cli::output::OutputFormat;

#[derive(Args, Debug, Clone)]
pub struct ChatArgs {
    /// Prompt for the agent plan
    #[arg(long, conflicts_with = "prompt_file")]
    pub prompt: Option<String>,

    /// Read prompt from file
    #[arg(long, conflicts_with = "prompt")]
    pub prompt_file: Option<PathBuf>,

    /// Provide initial constraints
    #[arg(long = "constraint", value_name = "TEXT", alias = "constraints")]
    pub constraints: Vec<String>,

    /// Current session URL
    #[arg(long, value_name = "URL")]
    pub current_url: Option<String>,

    /// Execute the generated plan immediately via the scheduler
    #[arg(long)]
    pub execute: bool,

    /// Maximum retry attempts per tool when executing (0 = no retry)
    #[arg(long, default_value_t = 1)]
    pub max_retries: u8,

    /// Maximum number of replanning attempts after execution failure
    #[arg(long, default_value_t = 0)]
    pub max_replans: u8,

    /// Persist combined plan/execution run metadata to a file (JSON)
    #[arg(long, value_name = "PATH")]
    pub save_run: Option<PathBuf>,

    /// Emit only the artifact manifest when using JSON/YAML output
    #[arg(long)]
    pub artifacts_only: bool,

    /// Write the artifact manifest to a JSON file
    #[arg(long, value_name = "PATH")]
    pub artifacts_path: Option<PathBuf>,

    /// Persist the generated plan to a JSON file
    #[arg(long, value_name = "PATH")]
    pub save_plan: Option<PathBuf>,

    /// Persist the generated flow graph to a JSON file
    #[arg(long, value_name = "PATH")]
    pub save_flow: Option<PathBuf>,

    /// Provide an input selector for the demo helpers
    #[arg(long, default_value = "input[type=text]")]
    pub input_selector: String,

    /// Provide an input payload for the demo helpers
    #[arg(long, default_value = "hello world")]
    pub input_text: String,

    /// Provide a submit selector for the demo helpers
    #[arg(long, default_value = "button[type=submit]")]
    pub submit_selector: String,

    /// Skip submitting the form during the demo helpers
    #[arg(long)]
    pub skip_submit: bool,

    /// Planner to use (rule or llm)
    #[arg(long, value_name = "PLANNER")]
    pub planner: Option<String>,

    /// LLM provider to power planning (openai, anthropic, mock)
    #[arg(long, value_name = "PROVIDER")]
    pub llm_provider: Option<String>,

    /// Override LLM model identifier
    #[arg(long, value_name = "MODEL")]
    pub llm_model: Option<String>,

    /// Override LLM API base URL
    #[arg(long, value_name = "URL")]
    pub llm_api_base: Option<String>,

    /// Override LLM temperature
    #[arg(long, value_name = "TEMP")]
    pub llm_temperature: Option<f32>,

    /// Override LLM API key (falls back to env vars)
    #[arg(long, value_name = "KEY")]
    pub llm_api_key: Option<String>,

    /// Override LLM max output tokens
    #[arg(long, value_name = "TOKENS")]
    pub llm_max_output_tokens: Option<u32>,
}

pub async fn cmd_chat(args: ChatArgs, ctx: &CliContext, output: OutputFormat) -> Result<()> {
    info!("Generating agent plan");

    let prompt = if let Some(prompt) = args.prompt.clone() {
        prompt
    } else if let Some(path) = args.prompt_file.as_ref() {
        fs::read_to_string(path)
            .await
            .with_context(|| format!("Failed to read prompt file: {}", path.display()))?
    } else {
        return Err(anyhow!("Either --prompt or --prompt-file must be provided"));
    };

    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(anyhow!("Prompt cannot be empty"));
    }

    let app_context = ctx.app_context().await?;

    let mut agent_context = AgentContext::default();
    if let Some(url) = args.current_url.clone() {
        agent_context.current_url = Some(url);
    }

    let has_context = agent_context.session.is_some()
        || agent_context.page.is_some()
        || agent_context.current_url.is_some()
        || !agent_context.preferences.is_empty()
        || !agent_context.memory_hints.is_empty()
        || !agent_context.metadata.is_empty();

    let planner_choice = match args.planner.as_deref().map(PlannerSelection::from_str_case) {
        Some(Some(kind)) => kind,
        Some(None) => bail!("Unknown planner specified"),
        None => PlannerSelection::Llm,
    };

    let llm_choice = match args
        .llm_provider
        .as_deref()
        .map(LlmProviderSelection::from_str_case)
    {
        Some(Some(kind)) => Some(kind),
        Some(None) => bail!("Unknown llm-provider specified"),
        None => None,
    };

    let llm_config = LlmProviderConfig {
        model: args.llm_model.clone(),
        api_base: args.llm_api_base.clone(),
        temperature: args.llm_temperature,
        api_key: args.llm_api_key.clone(),
        max_output_tokens: args.llm_max_output_tokens,
    };

    let ChatRunnerBuild {
        runner,
        planner_used: actual_planner,
        provider_used,
        fallback_reason,
    } = build_chat_runner(planner_choice, llm_choice, llm_config, None, None)?;

    let planner_warning = fallback_reason.clone();
    if let Some(note) = planner_warning.as_ref() {
        info!("Planner fallback: {note}");
    } else {
        let provider_label = provider_used.map(|p| p.label()).unwrap_or("-");
        info!(
            planner = actual_planner.label(),
            llm_provider = provider_label,
            "Planner configured"
        );
    }
    let mut exec_request = runner.request_from_prompt(
        prompt.clone(),
        if has_context {
            Some(agent_context)
        } else {
            None
        },
        args.constraints.clone(),
    );

    if let Some(note) = planner_warning {
        exec_request
            .metadata
            .insert("planner_warning".to_string(), Value::String(note));
    }

    exec_request.metadata.insert(
        "planner_used".to_string(),
        Value::String(actual_planner.label().to_string()),
    );

    if let Some(provider) = provider_used {
        exec_request.metadata.insert(
            "llm_provider".to_string(),
            Value::String(provider.label().to_string()),
        );
    }

    let mut current_session = runner.plan(exec_request.clone()).await?;

    let mut plan_payloads = Vec::new();
    let mut execution_reports = Vec::new();
    let mut state_events_payload: Option<Vec<Value>> = None;

    if let Some(path) = args.save_plan.as_ref() {
        let json = serde_json::to_string_pretty(&current_session.plan)?;
        fs::write(path, json)
            .await
            .with_context(|| format!("Failed to write plan to {}", path.display()))?;
        info!("Plan saved to: {}", path.display());
    }

    if let Some(path) = args.save_flow.as_ref() {
        let json = serde_json::to_string_pretty(&current_session.flow.flow)?;
        fs::write(path, json)
            .await
            .with_context(|| format!("Failed to write flow to {}", path.display()))?;
        info!("Flow saved to: {}", path.display());
    }

    let mut attempt = 0u32;

    loop {
        plan_payloads.push(plan_payload(&current_session));

        if matches!(output, OutputFormat::Human) {
            print_human_plan(&current_session, attempt);
        }

        if !args.execute {
            break;
        }

        let exec_options = FlowExecutionOptions {
            max_retries: args.max_retries.max(1),
            ..FlowExecutionOptions::default()
        };

        let exec_report = execute_plan(
            app_context.clone(),
            &exec_request,
            &current_session.plan,
            exec_options,
            None,
        )
        .await?;

        if matches!(output, OutputFormat::Human) {
            print_execution_summary(&exec_report, attempt);
        }

        execution_reports.push(exec_report.clone());
        state_events_payload = Some(state_events_to_values(&app_context.state_center_snapshot()));

        if exec_report.success {
            break;
        }

        if attempt >= args.max_replans.into() {
            if let Some(last) = exec_report
                .steps
                .iter()
                .rev()
                .find(|s| matches!(s.status, StepExecutionStatus::Failed))
            {
                bail!("Execution stopped at step '{}'", last.step_id);
            } else {
                bail!("Execution failed");
            }
        }

        if let Some(updated) = augment_request_for_replan(&exec_request, &exec_report, attempt) {
            exec_request = updated;
        }

        current_session = runner.plan(exec_request.clone()).await?;
        attempt += 1;
    }

    let state_events_slice = state_events_payload.as_ref().map(|v| v.as_slice());
    let (execution_payloads, manifest) =
        build_execution_payloads(&execution_reports, state_events_slice);

    if !matches!(output, OutputFormat::Human) {
        emit_structured_output(
            &plan_payloads,
            &execution_payloads,
            &manifest,
            state_events_slice,
            output.clone(),
            args.artifacts_only,
        )?;
    }

    if let Some(path) = args.artifacts_path.as_ref() {
        save_artifact_manifest(path, &manifest).await?;
        info!("Artifact manifest written to: {}", path.display());
    }

    if let Some(path) = args.save_run.as_ref() {
        persist_run(
            path,
            &plan_payloads,
            &execution_payloads,
            &manifest,
            state_events_slice,
        )
        .await?;
        info!("Run data saved to: {}", path.display());
    }

    Ok(())
}

fn print_human_plan(session: &ChatSessionOutput, attempt: u32) {
    println!("Agent Plan (attempt {})", attempt + 1);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    if !session.plan.meta.overlays.is_empty() {
        println!("Auto Repairs/Notes:");
        for overlay in &session.plan.meta.overlays {
            let title = overlay
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Auto repair");
            let detail = overlay.get("detail").and_then(Value::as_str).unwrap_or("");
            if detail.is_empty() {
                println!("  - {}", title);
            } else {
                println!("  - {}: {}", title, detail);
            }
        }
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    }
    for (idx, step) in session.plan.steps.iter().enumerate() {
        println!("{}. {}", idx + 1, step.title);
    }
}

fn print_execution_summary(report: &FlowExecutionReport, attempt: u32) {
    println!("Execution Summary (attempt {})", attempt + 1);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    for (idx, step) in report.steps.iter().enumerate() {
        let _status = match step.status {
            StepExecutionStatus::Success => "success",
            StepExecutionStatus::Failed => "failed",
        };
        println!(
            "  - Step {}: {} ({} attempt{})",
            idx + 1,
            step.title,
            step.attempts,
            if step.attempts == 1 { "" } else { "s" }
        );
        if let Some(error) = &step.error {
            println!("      error: {}", error);
        }
        for dispatch in &step.dispatches {
            println!(
                "      {}: action={} wait={}ms run={}ms",
                dispatch.label, dispatch.action_id, dispatch.wait_ms, dispatch.run_ms
            );
            println!(
                "         route: session={} page={} frame={} mutex={}",
                dispatch.route.session.0,
                dispatch.route.page.0,
                dispatch.route.frame.0,
                dispatch.route.mutex_key
            );
            if let Some(err) = &dispatch.error {
                println!("         error: {}", err);
            }
            for artifact in &dispatch.artifacts {
                println!(
                    "         artifact: {} ({} bytes, {}){}",
                    artifact.label,
                    artifact.byte_len,
                    artifact.content_type,
                    artifact
                        .filename
                        .as_ref()
                        .map(|name| format!(", filename={}", name))
                        .unwrap_or_default()
                );
            }
        }
    }
}

fn emit_structured_output(
    plans: &[Value],
    execution: &[Value],
    manifest: &ArtifactManifest,
    events: Option<&[Value]>,
    output: OutputFormat,
    artifacts_only: bool,
) -> Result<()> {
    if artifacts_only {
        return emit_artifact_manifest(manifest, output);
    }

    let payload = manifest.build_output_payload(plans, execution, events);

    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&payload)?),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&payload)?),
        OutputFormat::Human => {}
    }

    Ok(())
}

fn emit_artifact_manifest(manifest: &ArtifactManifest, output: OutputFormat) -> Result<()> {
    let artifacts_array = manifest.json_array();
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&artifacts_array)?),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&artifacts_array)?),
        OutputFormat::Human => {
            for record in &manifest.items {
                println!(
                    "attempt={} step={} ({}) dispatch={} artifact={} bytes={} type={}{}",
                    record.attempt,
                    record.step_index,
                    record.step_id,
                    record.dispatch_label,
                    record.label,
                    record.byte_len,
                    record.content_type,
                    record
                        .filename
                        .as_ref()
                        .map(|name| format!(" filename={}", name))
                        .unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}

fn build_execution_payloads(
    reports: &[FlowExecutionReport],
    events: Option<&[Value]>,
) -> (Vec<Value>, ArtifactManifest) {
    let mut payloads = Vec::new();
    let mut manifest = ArtifactManifest::default();
    for (attempt, report) in reports.iter().enumerate() {
        payloads.push(execution_report_payload(
            report,
            attempt as u32,
            events,
            &mut manifest,
        ));
    }
    (payloads, manifest)
}

fn execution_report_payload(
    report: &FlowExecutionReport,
    attempt: u32,
    events: Option<&[Value]>,
    manifest: &mut ArtifactManifest,
) -> Value {
    let mut steps = Vec::new();

    for (step_index, step) in report.steps.iter().enumerate() {
        let mut step_obj = serde_json::Map::new();
        step_obj.insert("step_id".into(), Value::String(step.step_id.clone()));
        step_obj.insert("title".into(), Value::String(step.title.clone()));
        step_obj.insert(
            "status".into(),
            Value::String(
                match step.status {
                    StepExecutionStatus::Success => "success",
                    StepExecutionStatus::Failed => "failed",
                }
                .to_string(),
            ),
        );
        step_obj.insert("attempts".into(), Value::from(step.attempts));
        step_obj.insert(
            "error".into(),
            step.error.clone().map(Value::String).unwrap_or(Value::Null),
        );

        let mut dispatch_values = Vec::new();
        for (dispatch_index, dispatch) in step.dispatches.iter().enumerate() {
            let mut dispatch_obj = serde_json::Map::new();
            dispatch_obj.insert("label".into(), Value::String(dispatch.label.clone()));
            dispatch_obj.insert(
                "action_id".into(),
                Value::String(dispatch.action_id.clone()),
            );
            dispatch_obj.insert(
                "route".into(),
                json!({
                    "session": dispatch.route.session.0,
                    "page": dispatch.route.page.0,
                    "frame": dispatch.route.frame.0,
                    "mutex": dispatch.route.mutex_key,
                }),
            );
            dispatch_obj.insert("wait_ms".into(), Value::from(dispatch.wait_ms));
            dispatch_obj.insert("run_ms".into(), Value::from(dispatch.run_ms));
            dispatch_obj.insert(
                "output".into(),
                dispatch.output.clone().unwrap_or(Value::Null),
            );

            let artifact_values: Vec<Value> = dispatch
                .artifacts
                .iter()
                .enumerate()
                .map(|(artifact_index, artifact)| {
                    manifest.add(ArtifactRecord {
                        attempt,
                        step_index,
                        step_id: step.step_id.clone(),
                        dispatch_label: dispatch.label.clone(),
                        dispatch_index,
                        artifact_index,
                        action_id: dispatch.action_id.clone(),
                        label: artifact.label.clone(),
                        content_type: artifact.content_type.clone(),
                        byte_len: artifact.byte_len,
                        filename: artifact.filename.clone(),
                        data_base64: artifact.data_base64.clone(),
                    });

                    json!({
                        "label": artifact.label,
                        "content_type": artifact.content_type,
                        "byte_len": artifact.byte_len,
                        "filename": artifact.filename,
                        "data_base64": artifact.data_base64,
                    })
                })
                .collect();
            dispatch_obj.insert("artifacts".into(), Value::Array(artifact_values));

            dispatch_values.push(Value::Object(dispatch_obj));
        }
        step_obj.insert("dispatches".into(), Value::Array(dispatch_values));

        steps.push(Value::Object(step_obj));
    }

    let mut payload = serde_json::Map::new();
    payload.insert("attempt".into(), Value::Number(attempt.into()));
    payload.insert("steps".into(), Value::Array(steps));
    if let Some(events) = events {
        payload.insert("state_events".into(), Value::Array(events.to_vec()));
    }

    Value::Object(payload)
}

fn augment_request_for_replan(
    request: &AgentRequest,
    report: &FlowExecutionReport,
    attempt: u32,
) -> Option<AgentRequest> {
    if report.steps.is_empty() {
        return None;
    }

    let last_step = report.steps.last()?;
    if !matches!(last_step.status, StepExecutionStatus::Failed) {
        return None;
    }

    let mut next_request = request.clone();
    next_request.constraints.push(format!(
        "Previous attempt {} failed at step '{}': {}",
        attempt + 1,
        last_step.step_id,
        last_step.error.as_deref().unwrap_or("unknown error")
    ));
    next_request.conversation.push(ConversationTurn::new(
        ConversationRole::User,
        "Please suggest a revised plan that can succeed.",
    ));

    Some(next_request)
}

#[derive(Default)]
struct ArtifactManifest {
    items: Vec<ArtifactRecord>,
}

impl ArtifactManifest {
    fn add(&mut self, record: ArtifactRecord) {
        self.items.push(record);
    }

    fn build_output_payload(
        &self,
        plans: &[Value],
        execution: &[Value],
        events: Option<&[Value]>,
    ) -> Value {
        let artifacts = self.json_array();
        build_payload(plans, execution, artifacts, events)
    }

    fn json_array(&self) -> Value {
        Value::Array(self.items.iter().map(|record| record.to_value()).collect())
    }
}

fn build_payload(
    plans: &[Value],
    execution: &[Value],
    artifacts: Value,
    events: Option<&[Value]>,
) -> Value {
    if let Some(events) = events {
        json!({
            "plans": plans,
            "execution": execution,
            "artifacts": artifacts,
            "state_events": events,
        })
    } else {
        json!({
            "plans": plans,
            "execution": execution,
            "artifacts": artifacts,
        })
    }
}

async fn persist_run(
    path: &PathBuf,
    plans: &[Value],
    execution: &[Value],
    manifest: &ArtifactManifest,
    events: Option<&[Value]>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    let payload = manifest.build_output_payload(plans, execution, events);
    fs::write(path, serde_json::to_vec_pretty(&payload)?)
        .await
        .with_context(|| format!("Failed to write run data to {}", path.display()))?;
    Ok(())
}

async fn save_artifact_manifest(path: &PathBuf, manifest: &ArtifactManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    let json_bytes = serde_json::to_vec_pretty(&manifest.json_array())?;
    fs::write(path, json_bytes)
        .await
        .with_context(|| format!("Failed to write artifact manifest to {}", path.display()))?;
    Ok(())
}

#[derive(Clone, Debug)]
struct ArtifactRecord {
    attempt: u32,
    step_index: usize,
    step_id: String,
    dispatch_label: String,
    dispatch_index: usize,
    artifact_index: usize,
    action_id: String,
    label: String,
    content_type: String,
    byte_len: usize,
    filename: Option<String>,
    data_base64: String,
}

impl ArtifactRecord {
    fn to_value(&self) -> Value {
        json!({
            "attempt": self.attempt,
            "step_index": self.step_index,
            "step_id": self.step_id,
            "dispatch_label": self.dispatch_label,
            "dispatch_index": self.dispatch_index,
            "artifact_index": self.artifact_index,
            "action_id": self.action_id,
            "label": self.label,
            "content_type": self.content_type,
            "byte_len": self.byte_len,
            "filename": self.filename,
            "data_base64": self.data_base64,
        })
    }
}

fn state_events_to_values(events: &[StateEvent]) -> Vec<Value> {
    events
        .iter()
        .map(|event| match event {
            StateEvent::Dispatch(dispatch) => json!({
                "type": "dispatch",
                "status": match dispatch.status {
                    DispatchStatus::Success => "success",
                    DispatchStatus::Failure => "failure",
                },
                "action_id": dispatch.action_id.0,
                "task_id": dispatch.task_id,
                "route": {
                    "session": dispatch.route.session.0,
                    "page": dispatch.route.page.0,
                    "frame": dispatch.route.frame.0,
                    "mutex": dispatch.route.mutex_key.clone(),
                },
                "tool": dispatch.tool,
                "attempts": dispatch.attempts,
                "wait_ms": dispatch.wait_ms,
                "run_ms": dispatch.run_ms,
                "pending": dispatch.pending,
                "slots_available": dispatch.slots_available,
                "error": dispatch.error.as_ref().map(|e| e.to_string()),
                "output": dispatch.output.clone(),
                "recorded_at_ms": system_time_ms(dispatch.recorded_at),
            }),
            StateEvent::Registry(registry) => json!({
                "type": "registry",
                "action": format!("{:?}", registry.action).to_lowercase(),
                "session": registry.session.as_ref().map(|s| s.0.clone()),
                "page": registry.page.as_ref().map(|p| p.0.clone()),
                "frame": registry.frame.as_ref().map(|f| f.0.clone()),
                "note": registry.note,
                "recorded_at_ms": system_time_ms(registry.recorded_at),
            }),
            StateEvent::Perceiver(perceiver) => {
                let details = match &perceiver.kind {
                    PerceiverEventKind::Resolve {
                        strategy,
                        score,
                        candidate_count,
                        cache_hit,
                        breakdown,
                        reason,
                    } => json!({
                        "kind": "resolve",
                        "strategy": strategy,
                        "score": score,
                        "candidate_count": candidate_count,
                        "cache_hit": cache_hit,
                        "breakdown": breakdown,
                        "reason": reason,
                    }),
                    PerceiverEventKind::Judge {
                        check,
                        ok,
                        reason,
                        facts,
                    } => json!({
                        "kind": "judge",
                        "check": check,
                        "ok": ok,
                        "reason": reason,
                        "facts": facts,
                    }),
                    PerceiverEventKind::Snapshot { cache_hit } => json!({
                        "kind": "snapshot",
                        "cache_hit": cache_hit,
                    }),
                    PerceiverEventKind::Diff {
                        change_count,
                        changes,
                    } => json!({
                        "kind": "diff",
                        "change_count": change_count,
                        "changes": changes,
                    }),
                };

                json!({
                    "type": "perceiver",
                    "route": {
                        "session": perceiver.route.session.0.clone(),
                        "page": perceiver.route.page.0.clone(),
                        "frame": perceiver.route.frame.0.clone(),
                        "mutex": perceiver.route.mutex_key.clone(),
                    },
                    "recorded_at_ms": system_time_ms(perceiver.recorded_at),
                    "details": details,
                })
            }
        })
        .collect()
}

fn system_time_ms(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .map(|dur| dur.as_millis())
        .unwrap_or(0)
}
