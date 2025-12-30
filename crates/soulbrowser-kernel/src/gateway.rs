use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_core::{AgentContext, AgentPlan, AgentRequest};
use anyhow::{anyhow, Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use l7_adapter::ports::{NoopReadonly, SchedulerDispatcher};
use l7_adapter::{AdapterBootstrap, AdapterPolicyHandle, AdapterPolicyView, TenantPolicy};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::fs;
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::sync::broadcast;
use tokio::time::{timeout, Duration};
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};

use crate::agent::{execute_plan, FlowExecutionOptions};
use crate::app_context::AppContext;
use crate::gateway_policy::{gateway_auth_middleware, GatewayPolicy};
use crate::kernel::Kernel;
use crate::runtime::RuntimeOptions;
use crate::task_status::{TaskStatusRegistry, TaskStreamEnvelope};
use crate::visualization::build_plan_overlays;
use soulbrowser_core_types::TaskId;

#[derive(Clone, Debug)]
pub struct GatewayOptions {
    pub http: SocketAddr,
    pub grpc: Option<SocketAddr>,
    pub webdriver: Option<SocketAddr>,
    pub adapter_policy: Option<PathBuf>,
    pub webdriver_policy: Option<PathBuf>,
    pub demo_plan: Option<PathBuf>,
    pub runtime: RuntimeOptions,
}

pub async fn run_gateway(kernel: &Kernel, options: GatewayOptions) -> Result<()> {
    let runtime = kernel.start_runtime(options.runtime.clone()).await?;
    let context = runtime.state.app_context().await;

    let adapter_policy = load_adapter_policy(options.adapter_policy.clone())?;
    let adapter_handle = AdapterPolicyHandle::global();
    adapter_handle.update(adapter_policy.clone());
    info!(
        enabled = adapter_policy.enabled,
        tenants = adapter_policy.tenants.len(),
        "Adapter policy loaded"
    );

    let scheduler_service = context.scheduler_service();
    let dispatcher_port: Arc<dyn l7_adapter::ports::DispatcherPort> =
        Arc::new(SchedulerDispatcher::new(scheduler_service));
    let readonly_port: Arc<dyn l7_adapter::ports::ReadonlyPort> = Arc::new(NoopReadonly);
    let bootstrap = AdapterBootstrap::new(adapter_handle, dispatcher_port, readonly_port);

    let edge_policy = Arc::new(load_gateway_policy(options.adapter_policy.clone())?);
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

    let shared_state = GatewayState {
        context: context.clone(),
        registry: context.task_status_registry(),
    };

    let adapter_router = bootstrap.build_http();
    let custom_router = Router::new()
        .route(
            "/v1/tasks/:task_id/stream",
            get(gateway_task_stream_handler),
        )
        .route("/v1/tasks/run", post(gateway_run_handler))
        .with_state(shared_state.clone());

    let router = Router::new()
        .merge(adapter_router)
        .merge(custom_router)
        .layer(cors)
        .layer(middleware::from_fn_with_state(
            edge_policy.clone(),
            gateway_auth_middleware,
        ));

    let listener = TcpListener::bind(options.http)
        .await
        .with_context(|| format!("failed to bind gateway http on {}", options.http))?;
    info!("Gateway HTTP ready at http://{}", options.http);

    if let Some(plan_path) = options.demo_plan.clone() {
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

    if let Some(addr) = options.grpc {
        warn!(%addr, "gRPC adapter requested but not implemented in this build");
    }

    if let Some(addr) = options.webdriver {
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

#[derive(Clone)]
struct GatewayState {
    context: Arc<AppContext>,
    registry: Arc<TaskStatusRegistry>,
}

async fn gateway_task_stream_handler(
    AxumPath(task_id): AxumPath<String>,
    State(state): State<GatewayState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    upgrade_task_stream(state.registry, task_id, ws)
}

#[derive(Deserialize)]
struct GatewayRunRequest {
    prompt: String,
    plan: AgentPlan,
    constraints: Vec<String>,
    #[serde(default)]
    context: Option<AgentContext>,
}

#[derive(Serialize)]
struct GatewayRunResponse {
    task_id: TaskId,
    overlays: Value,
}

async fn gateway_run_handler(
    State(state): State<GatewayState>,
    Json(payload): Json<GatewayRunRequest>,
) -> Result<Json<GatewayRunResponse>, (StatusCode, String)> {
    let mut agent_request = AgentRequest::new(payload.plan.task_id.clone(), payload.prompt);
    agent_request.constraints = payload.constraints;
    if let Some(ctx) = payload.context {
        agent_request = agent_request.with_context(ctx);
    }
    let task_id = agent_request.task_id.clone();
    let overlays = build_plan_overlays(&payload.plan);
    spawn_gateway_execution(state.context.clone(), agent_request, payload.plan.clone());

    Ok(Json(GatewayRunResponse { task_id, overlays }))
}

fn upgrade_task_stream(
    registry: Arc<TaskStatusRegistry>,
    task_id: String,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |mut socket| async move {
        let Some(_) = registry.snapshot(&task_id) else {
            return;
        };
        let history = registry
            .stream_history_since(&task_id, None)
            .unwrap_or_default();
        let mut receiver = match registry.subscribe(&task_id) {
            Some(rx) => rx,
            None => return,
        };

        for envelope in history {
            if send_stream_event(&mut socket, &envelope).await.is_err() {
                return;
            }
        }

        loop {
            tokio::select! {
                event = receiver.recv() => {
                    match event {
                        Ok(envelope) => {
                            if send_stream_event(&mut socket, &envelope).await.is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = timeout(Duration::from_secs(60), async {}) => {
                    if send_ping(&mut socket).await.is_err() {
                        break;
                    }
                }
            }
        }
    })
}

async fn send_stream_event(socket: &mut WebSocket, envelope: &TaskStreamEnvelope) -> Result<()> {
    let payload = serde_json::to_string(envelope)
        .map_err(|err| anyhow!("failed to serialize task stream: {err}"))?;
    socket
        .send(Message::Text(payload))
        .await
        .map_err(|err| anyhow!("failed to send task stream WS message: {err}"))
}

async fn send_ping(socket: &mut WebSocket) -> Result<()> {
    socket
        .send(Message::Ping(vec![]))
        .await
        .map_err(|err| anyhow!("failed to send ping: {err}"))
}

fn load_gateway_policy(path: Option<PathBuf>) -> Result<GatewayPolicy> {
    let Some(path) = path else {
        return Ok(GatewayPolicy::default());
    };

    let bytes = std::fs::read(&path)?;
    match parse_policy_bytes::<RawGatewayPolicy>(&bytes, &path, "gateway policy") {
        Ok(raw) => {
            let ips = raw
                .ip_whitelist
                .into_iter()
                .filter_map(|value| match value.parse::<IpAddr>() {
                    Ok(addr) => Some(addr),
                    Err(err) => {
                        warn!(%value, ?err, "invalid IP in gateway policy");
                        None
                    }
                })
                .collect();
            Ok(GatewayPolicy::from_tokens_and_ips(raw.allowed_tokens, ips))
        }
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

async fn run_gateway_demo_plan(plan_path: PathBuf, context: Arc<AppContext>) -> Result<TaskId> {
    let bytes = fs::read(&plan_path)
        .await
        .with_context(|| format!("reading gateway demo plan {}", plan_path.display()))?;

    let payload: GatewayRunRequest = serde_json::from_slice(&bytes)
        .or_else(|_| serde_yaml::from_slice(&bytes))
        .with_context(|| format!("parsing gateway demo plan {}", plan_path.display()))?;

    let mut agent_request = AgentRequest::new(payload.plan.task_id.clone(), payload.prompt);
    agent_request.constraints = payload.constraints;
    if let Some(ctx) = payload.context {
        agent_request = agent_request.with_context(ctx);
    }

    let task_id = agent_request.task_id.clone();
    spawn_gateway_execution(context, agent_request, payload.plan);
    Ok(task_id)
}

fn spawn_gateway_execution(context: Arc<AppContext>, request: AgentRequest, plan: AgentPlan) {
    let opts = FlowExecutionOptions::default();
    spawn(async move {
        let task_id = request.task_id.clone();
        let task_label = task_id.0.clone();
        match execute_plan(context, &request, &plan, opts, None).await {
            Ok(report) => {
                info!(task = %task_label, success = report.success, "Gateway plan executed");
            }
            Err(err) => {
                error!(task = %task_label, ?err, "Gateway plan execution failed");
            }
        }
    });
}

#[derive(Deserialize)]
struct RawGatewayPolicy {
    allowed_tokens: Vec<String>,
    ip_whitelist: Vec<String>,
}
