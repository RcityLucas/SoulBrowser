use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{ws::WebSocketUpgrade, Path as AxumPath, State},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use clap::Args;
use l7_adapter::http::AdapterState;
use l7_adapter::ports::{NoopReadonly, SchedulerDispatcher};
use l7_adapter::{AdapterBootstrap, AdapterPolicyHandle, AdapterPolicyView, TenantPolicy};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::spawn;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};

use crate::agent::{execute_plan, FlowExecutionOptions};
use crate::app_context::{get_or_create_context, AppContext};
use crate::bind_tcp_listener;
use crate::gateway_auth_middleware;
use crate::run_gateway_demo_plan;
use crate::task_status::TaskStatusRegistry;
use crate::upgrade_task_stream;
use crate::visualization::build_plan_overlays;
use crate::Config;
use crate::GatewayPolicy;
use agent_core::{AgentContext, AgentPlan, AgentRequest};

#[derive(Args, Clone)]
pub struct GatewayArgs {
    /// HTTP listener address (host:port)
    #[arg(long, default_value = "127.0.0.1:8710")]
    pub http: SocketAddr,

    /// Optional gRPC listener (not yet implemented)
    #[arg(long)]
    pub grpc: Option<SocketAddr>,

    /// Optional WebDriver bridge listener (not yet implemented)
    #[arg(long)]
    pub webdriver: Option<SocketAddr>,

    /// Path to adapter policy definition (json/yaml)
    #[arg(long, value_name = "FILE")]
    pub adapter_policy: Option<PathBuf>,

    /// Path to WebDriver bridge policy definition (json/yaml)
    #[arg(long, value_name = "FILE")]
    pub webdriver_policy: Option<PathBuf>,

    /// Optional path to task plan JSON to run immediately when gateway starts
    #[arg(long, value_name = "FILE")]
    pub demo_plan: Option<PathBuf>,
}

#[derive(Clone)]
pub(crate) struct GatewayState {
    pub(crate) adapter: AdapterState,
    pub(crate) stream: GatewayStreamState,
    pub(crate) context: Arc<AppContext>,
}

#[derive(Clone)]
pub(crate) struct GatewayStreamState {
    pub(crate) registry: Arc<TaskStatusRegistry>,
}

impl axum::extract::FromRef<GatewayState> for AdapterState {
    fn from_ref(state: &GatewayState) -> Self {
        state.adapter.clone()
    }
}

impl axum::extract::FromRef<GatewayState> for GatewayStreamState {
    fn from_ref(state: &GatewayState) -> Self {
        state.stream.clone()
    }
}

pub async fn cmd_gateway(args: GatewayArgs, config: &Config) -> Result<()> {
    let context = get_gateway_context(config).await?;

    let adapter_policy = load_adapter_policy(args.adapter_policy.clone())?;
    let adapter_handle = AdapterPolicyHandle::global();
    adapter_handle.update(adapter_policy.clone());
    info!(
        enabled = adapter_policy.enabled,
        tenants = adapter_policy.tenants.len(),
        "Adapter policy loaded"
    );

    let scheduler_service = context.scheduler_service();
    let dispatcher_inner: Arc<dyn soulbrowser_scheduler::Dispatcher> = scheduler_service;
    let dispatcher_port: Arc<dyn l7_adapter::ports::DispatcherPort> =
        Arc::new(SchedulerDispatcher::new(dispatcher_inner));
    let readonly_port: Arc<dyn l7_adapter::ports::ReadonlyPort> = Arc::new(NoopReadonly);
    let bootstrap = AdapterBootstrap::new(adapter_handle, dispatcher_port, readonly_port);

    let edge_policy = Arc::new(load_gateway_policy(args.adapter_policy.clone())?);
    if !edge_policy.allowed_tokens.is_empty() {
        info!(
            count = edge_policy.allowed_tokens.len(),
            "Gateway token whitelist active"
        );
    }
    if !edge_policy.ip_whitelist.is_empty() {
        info!(
            count = edge_policy.ip_whitelist.len(),
            "Gateway IP whitelist active"
        );
    }

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers(Any);

    let adapter_state = bootstrap.state();
    let stream_state = GatewayStreamState {
        registry: context.task_status_registry(),
    };
    let gateway_state = GatewayState {
        adapter: adapter_state.clone(),
        stream: stream_state.clone(),
        context: context.clone(),
    };

    let adapter_router: Router<GatewayState> = bootstrap
        .build_http()
        .with_state::<GatewayState>(adapter_state.clone());
    let stream_router: Router<GatewayState> = Router::new()
        .route(
            "/v1/tasks/:task_id/stream",
            get(gateway_task_stream_handler),
        )
        .with_state::<GatewayState>(stream_state.clone());

    let router = Router::new()
        .merge(adapter_router)
        .merge(stream_router)
        .route("/v1/tasks/run", post(gateway_run_handler))
        .layer(cors)
        .layer(middleware::from_fn_with_state(
            edge_policy.clone(),
            gateway_auth_middleware,
        ))
        .with_state(gateway_state);

    let listener = bind_tcp_listener(args.http, "gateway http")?;
    info!("Gateway HTTP ready at http://{}", args.http);

    if let Some(plan_path) = args.demo_plan.clone() {
        let ctx = context.clone();
        spawn(async move {
            match run_gateway_demo_plan(plan_path.clone(), ctx.clone()).await {
                Ok(task_id) => info!(
                    task = ?task_id,
                    path = %plan_path.display(),
                    "gateway demo plan executed"
                ),
                Err(err) => error!(
                    ?err,
                    path = %plan_path.display(),
                    "gateway demo plan failed"
                ),
            }
        });
    }

    if let Some(addr) = args.grpc {
        warn!(%addr, "gRPC adapter requested but not implemented in this build");
    }

    if let Some(addr) = args.webdriver {
        warn!(%addr, "WebDriver bridge requested but not implemented in this build");
    }

    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("gateway http server exited unexpectedly")?;
    Ok(())
}

#[derive(Deserialize)]
struct GatewayRunRequest {
    plan: AgentPlan,
    prompt: String,
    #[serde(default)]
    constraints: Vec<String>,
    #[serde(default)]
    context: Option<AgentContext>,
}

#[derive(Serialize)]
struct GatewayRunResponse {
    success: bool,
    task_id: String,
    stream_path: String,
}

async fn gateway_task_stream_handler(
    State(state): State<GatewayStreamState>,
    AxumPath(task_id): AxumPath<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    upgrade_task_stream(state.registry.clone(), task_id, ws)
}

async fn gateway_run_handler(
    State(state): State<GatewayState>,
    Json(payload): Json<GatewayRunRequest>,
) -> impl IntoResponse {
    let GatewayRunRequest {
        plan,
        prompt,
        constraints,
        context: ctx,
    } = payload;
    if plan.steps.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "error": "plan must contain at least one step",
            })),
        )
            .into_response();
    }

    let mut agent_request = AgentRequest::new(plan.task_id.clone(), prompt);
    agent_request.constraints = constraints;
    if let Some(ctx) = ctx {
        agent_request = agent_request.with_context(ctx);
    }

    let task_id = agent_request.task_id.clone();
    let task_id_str = task_id.0.clone();
    spawn_gateway_execution(state.context.clone(), agent_request, plan);

    (
        StatusCode::ACCEPTED,
        Json(GatewayRunResponse {
            success: true,
            task_id: task_id_str.clone(),
            stream_path: format!("/v1/tasks/{}/stream", task_id_str),
        }),
    )
        .into_response()
}

fn spawn_gateway_execution(context: Arc<AppContext>, request: AgentRequest, plan: AgentPlan) {
    spawn(async move {
        let status_registry = context.task_status_registry();
        let status_handle = status_registry.register(
            request.task_id.clone(),
            plan.title.clone(),
            plan.steps.len(),
        );
        status_handle.set_plan_overlays(build_plan_overlays(&plan));

        let heal_registry = context.self_heal_registry();
        let plugin_registry = context.plugin_registry();
        let exec_options = FlowExecutionOptions {
            max_retries: 1_u8
                .saturating_add(heal_registry.auto_retry_extra_attempts())
                .max(1),
            progress: Some(status_handle.clone()),
            self_heal: Some(heal_registry),
            plugin_registry: Some(plugin_registry),
            ..FlowExecutionOptions::default()
        };

        match execute_plan(context.clone(), &request, &plan, exec_options).await {
            Ok(report) => {
                if report.success {
                    info!(task = %request.task_id.0, "gateway plan completed");
                } else {
                    warn!(task = %request.task_id.0, "gateway plan finished with errors");
                }
            }
            Err(err) => {
                status_handle.mark_failure(Some(err.to_string()));
                error!(task = %request.task_id.0, ?err, "gateway plan execution failed");
            }
        }
    });
}

async fn get_gateway_context(config: &Config) -> Result<Arc<AppContext>> {
    get_or_create_context(
        "cli-gateway".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await
    .map_err(|err| anyhow!(err.to_string()))
}

fn load_gateway_policy(path: Option<PathBuf>) -> Result<GatewayPolicy> {
    let Some(path) = path else {
        return Ok(GatewayPolicy::default());
    };

    let bytes = std::fs::read(&path)?;
    match parse_policy_bytes::<RawGatewayPolicy>(&bytes, &path, "gateway policy") {
        Ok(raw) => Ok(GatewayPolicy::from_raw(raw)),
        Err(err) => {
            warn!(
                ?err,
                path = %path.display(),
                "failed to parse gateway policy; falling back to defaults"
            );
            Ok(GatewayPolicy::default())
        }
    }
}

fn load_adapter_policy(path: Option<PathBuf>) -> Result<AdapterPolicyView> {
    let Some(path) = path else {
        warn!("adapter policy not provided; falling back to default");
        return Ok(default_adapter_policy());
    };

    let bytes = std::fs::read(&path)?;
    let mut view = match parse_policy_bytes::<AdapterPolicyView>(&bytes, &path, "adapter policy") {
        Ok(view) => view,
        Err(err) => {
            warn!(?err, path = %path.display(), "failed to parse adapter policy; using defaults");
            default_adapter_policy()
        }
    };

    if policy_is_empty(&view) {
        warn!(path = %path.display(), "adapter policy empty; using defaults");
        view = default_adapter_policy();
    }

    if let Some(raw) = try_parse_gateway_policy_bytes(&bytes) {
        info!(
            path = %path.display(),
            tokens = raw.allowed_tokens.len(),
            ips = raw.ip_whitelist.len(),
            "adapter policy contained gateway auth section"
        );
    }

    normalize_adapter_policy(&mut view);
    Ok(view)
}

fn parse_policy_bytes<T>(bytes: &[u8], path: &Path, label: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());

    let parse_json = || {
        serde_json::from_slice(bytes)
            .with_context(|| format!("parsing {label} {} as json", path.display()))
    };
    let parse_yaml = || {
        serde_yaml::from_slice(bytes)
            .with_context(|| format!("parsing {label} {} as yaml", path.display()))
    };

    match ext.as_deref() {
        Some("json") => parse_json(),
        Some("yaml") | Some("yml") => parse_yaml(),
        _ => parse_json().or_else(|_| parse_yaml()),
    }
}

fn try_parse_gateway_policy_bytes(bytes: &[u8]) -> Option<RawGatewayPolicy> {
    serde_json::from_slice(bytes)
        .ok()
        .or_else(|| serde_yaml::from_slice(bytes).ok())
}

fn normalize_adapter_policy(view: &mut AdapterPolicyView) {
    if view.tenants.is_empty() {
        view.tenants.push(default_tenant_policy());
    }

    for tenant in &mut view.tenants {
        if tenant.id.trim().is_empty() {
            tenant.id = "default".into();
        }
        if tenant.timeout_ms_tool == 0 {
            tenant.timeout_ms_tool = DEFAULT_TOOL_TIMEOUT_MS;
        }
        if tenant.timeout_ms_flow == 0 {
            tenant.timeout_ms_flow = DEFAULT_FLOW_TIMEOUT_MS;
        }
        if tenant.timeout_ms_read == 0 {
            tenant.timeout_ms_read = DEFAULT_READ_TIMEOUT_MS;
        }
        if tenant.exports_max_lines == 0 {
            tenant.exports_max_lines = DEFAULT_EXPORT_MAX_LINES;
        }
    }

    if view.tracing_sample == 0.0 {
        view.tracing_sample = 1.0;
    }
}

fn policy_is_empty(view: &AdapterPolicyView) -> bool {
    !view.enabled
        && view.tenants.is_empty()
        && !view.cors_enable
        && !view.tls_required
        && view.privacy_profile.is_none()
        && view.tracing_sample == 0.0
}

fn default_adapter_policy() -> AdapterPolicyView {
    AdapterPolicyView {
        enabled: true,
        cors_enable: true,
        tracing_sample: 1.0,
        tenants: vec![default_tenant_policy()],
        ..AdapterPolicyView::default()
    }
}

fn default_tenant_policy() -> TenantPolicy {
    TenantPolicy {
        id: "default".into(),
        timeout_ms_tool: DEFAULT_TOOL_TIMEOUT_MS,
        timeout_ms_flow: DEFAULT_FLOW_TIMEOUT_MS,
        timeout_ms_read: DEFAULT_READ_TIMEOUT_MS,
        exports_max_lines: DEFAULT_EXPORT_MAX_LINES,
        allow_cold_export: false,
        ..TenantPolicy::default()
    }
}

const DEFAULT_TOOL_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_FLOW_TIMEOUT_MS: u64 = 180_000;
const DEFAULT_READ_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_EXPORT_MAX_LINES: usize = 10_000;

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct RawGatewayPolicy {
    #[serde(default)]
    pub(crate) allowed_tokens: Vec<String>,
    #[serde(default)]
    pub(crate) ip_whitelist: Vec<String>,
}
