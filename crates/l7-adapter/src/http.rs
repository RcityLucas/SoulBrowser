use crate::auth;
use crate::errors::AdapterError;
use crate::events::{AdapterResponseEvent, EventsPort};
use crate::guard::RequestGuard;
use crate::idempotency::IdempotencyStore;
use crate::policy::{AdapterPolicyHandle, AdapterPolicyView};
use crate::ports::{DispatcherPort, ReadonlyPort, ToolCall};
use crate::privacy;
use crate::trace::AdapterTracer;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use tracing::instrument;

#[derive(Clone)]
pub struct AdapterState {
    policy: AdapterPolicyHandle,
    dispatcher: Arc<dyn DispatcherPort>,
    #[allow(dead_code)]
    readonly: Arc<dyn ReadonlyPort>,
    guard: Arc<RequestGuard>,
    events: Arc<dyn EventsPort>,
    tracer: AdapterTracer,
    idempotency: Arc<IdempotencyStore>,
}

impl AdapterState {
    pub fn new(
        policy: AdapterPolicyHandle,
        dispatcher: Arc<dyn DispatcherPort>,
        readonly: Arc<dyn ReadonlyPort>,
        guard: Arc<RequestGuard>,
        events: Arc<dyn EventsPort>,
        tracer: AdapterTracer,
        idempotency: Arc<IdempotencyStore>,
    ) -> Self {
        Self {
            policy,
            dispatcher,
            readonly,
            guard,
            events,
            tracer,
            idempotency,
        }
    }

    pub(crate) fn snapshot(&self) -> AdapterPolicyView {
        self.policy.snapshot()
    }

    pub(crate) fn dispatcher(&self) -> Arc<dyn DispatcherPort> {
        Arc::clone(&self.dispatcher)
    }

    pub(crate) fn guard(&self) -> Arc<RequestGuard> {
        Arc::clone(&self.guard)
    }

    pub(crate) fn events(&self) -> Arc<dyn EventsPort> {
        Arc::clone(&self.events)
    }

    pub(crate) fn tracer(&self) -> AdapterTracer {
        self.tracer.clone()
    }

    pub(crate) fn idempotency(&self) -> Arc<IdempotencyStore> {
        Arc::clone(&self.idempotency)
    }
}

pub fn router_with_state(state: AdapterState) -> Router {
    Router::new()
        .route("/healthz", get(healthz_handler))
        .route("/v1/tools/run", post(run_tool_handler))
        .with_state(state)
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ToolRunRequest {
    pub tool: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub routing: Value,
    #[serde(default)]
    pub options: Value,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolRunResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
}

const DEFAULT_TIMEOUT_MS: u64 = 10_000;

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_MS
}

pub fn router(
    policy: AdapterPolicyHandle,
    dispatcher: Arc<dyn DispatcherPort>,
    readonly: Arc<dyn ReadonlyPort>,
    events: Arc<dyn EventsPort>,
    tracer: AdapterTracer,
    idempotency: Arc<IdempotencyStore>,
) -> Router {
    let guard = Arc::new(RequestGuard::new());
    let state = AdapterState::new(
        policy,
        dispatcher,
        readonly,
        guard,
        events,
        tracer,
        idempotency,
    );
    router_with_state(state)
}

async fn healthz_handler() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

#[instrument(skip_all, fields(tool = %req.tool))]
async fn run_tool_handler(
    State(state): State<AdapterState>,
    headers: HeaderMap,
    Json(req): Json<ToolRunRequest>,
) -> Result<Json<ToolRunResponse>, HttpError> {
    let policy = state.snapshot();
    if !policy.enabled {
        return Err(HttpError::service_unavailable("adapter disabled"));
    }

    let tenant_id = headers
        .get("x-tenant-id")
        .and_then(|value| value.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| HttpError::unauthorized("missing x-tenant-id header"))?;

    let tenant = policy
        .tenant(&tenant_id)
        .ok_or_else(|| HttpError::forbidden("tenant not permitted"))?;

    let payload_json = serde_json::to_string(&req)
        .map_err(|_| HttpError::invalid_argument("invalid request payload"))?;

    auth::verify_http(&headers, tenant, &payload_json).map_err(HttpError::unauthorized)?;

    if !tenant.allow_tools.is_empty() && !tenant.allow_tools.iter().any(|t| t == &req.tool) {
        return Err(HttpError::forbidden("tool not allowed"));
    }

    let timeout_ms = req.timeout_ms.min(tenant.timeout_ms_tool.max(1));

    let mut call = ToolCall {
        tenant_id: tenant_id.clone(),
        tool: req.tool.clone(),
        params: req.params,
        routing: req.routing,
        options: req.options,
        timeout_ms,
        idempotency_key: req.idempotency_key,
        trace_id: req.trace_id.or_else(|| Some(new_trace_id())),
    };

    privacy::sanitize_tool_call(&mut call);

    state.events.on_request(&call);

    let idempotency_ttl = if tenant.idempotency_window_sec == 0 {
        None
    } else {
        Some(Duration::from_secs(tenant.idempotency_window_sec))
    };
    if idempotency_ttl.is_some() {
        if let Some(key) = call.idempotency_key.as_ref() {
            if let Some(cached) = state.idempotency().lookup(&call.tenant_id, key) {
                state.events.on_response(&call, &cached);
                state.events.adapter_response(AdapterResponseEvent {
                    tenant_id: call.tenant_id.clone(),
                    tool: call.tool.clone(),
                    trace_id: cached.trace_id.clone(),
                    action_id: cached.action_id.clone(),
                    latency_ms: Some(0),
                    status: cached.status.clone(),
                    timestamp: Some(std::time::SystemTime::now()),
                });
                let body = ToolRunResponse {
                    status: cached.status,
                    data: cached.data,
                    trace_id: cached.trace_id,
                    action_id: cached.action_id,
                };
                return Ok(Json(body));
            }
        }
    }

    let span = state.tracer.span(&call.tenant_id, &call.tool);
    let _enter = span.enter();

    let _permit = state.guard.enter(tenant).map_err(HttpError::from)?;

    let started = OffsetDateTime::now_utc();
    let mut outcome = state
        .dispatcher
        .run_tool(call.clone())
        .await
        .map_err(HttpError::from)?;

    privacy::sanitize_tool_outcome(&call, &mut outcome);

    let elapsed = (OffsetDateTime::now_utc() - started).whole_milliseconds() as u128;
    state.events.on_response(&call, &outcome);
    state.events.adapter_response(AdapterResponseEvent {
        tenant_id: call.tenant_id.clone(),
        tool: call.tool.clone(),
        trace_id: outcome.trace_id.clone(),
        action_id: outcome.action_id.clone(),
        latency_ms: Some(elapsed),
        status: outcome.status.clone(),
        timestamp: Some(std::time::SystemTime::now()),
    });

    if let Some(ttl) = idempotency_ttl {
        if let Some(key) = call.idempotency_key.as_ref() {
            state
                .idempotency()
                .insert(&call.tenant_id, key.clone(), ttl, &outcome);
        }
    }

    let body = ToolRunResponse {
        status: outcome.status,
        data: outcome.data,
        trace_id: outcome.trace_id,
        action_id: outcome.action_id,
    };
    Ok(Json(body))
}

fn new_trace_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug)]
pub struct HttpError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl HttpError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "disabled", message)
    }

    fn too_many(message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "too_many_requests", message)
    }

    fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_argument", message)
    }

    fn not_implemented() -> Self {
        Self::new(
            StatusCode::NOT_IMPLEMENTED,
            "not_implemented",
            "not implemented",
        )
    }

    fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error",
        )
    }
}

impl From<AdapterError> for HttpError {
    fn from(value: AdapterError) -> Self {
        match value {
            AdapterError::NotImplemented(_) => HttpError::not_implemented(),
            AdapterError::Disabled => HttpError::service_unavailable("adapter disabled"),
            AdapterError::UnauthorizedTenant
            | AdapterError::TenantNotFound
            | AdapterError::ToolNotAllowed => HttpError::forbidden("operation not permitted"),
            AdapterError::TooManyRequests | AdapterError::ConcurrencyLimit => {
                HttpError::too_many("rate limit exceeded")
            }
            AdapterError::InvalidArgument => HttpError::invalid_argument("invalid argument"),
            AdapterError::Internal => HttpError::internal(),
        }
    }
}

impl axum::response::IntoResponse for HttpError {
    fn into_response(self) -> axum::response::Response {
        let body = Json(json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        }));
        axum::response::IntoResponse::into_response((self.status, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::AdapterResult;
    use crate::idempotency::IdempotencyStore;
    use crate::policy::{set_policy, AdapterPolicyView, TenantPolicy};
    use crate::ports::{NoopDispatcher, NoopReadonly, ToolOutcome};
    use axum::http::Request;
    use hex;
    use hmac::{Hmac, Mac};
    use serial_test::serial;
    use sha2::Sha256;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use time::OffsetDateTime;
    use tower::ServiceExt;

    #[tokio::test]
    #[serial]
    async fn healthz_is_ok() {
        let router = test_router();
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    #[serial]
    async fn disabled_adapter_returns_503() {
        set_policy(AdapterPolicyView::default());
        let router = test_router();
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tools/run")
                    .header("content-type", "application/json")
                    .header("x-tenant-id", "tenant-1")
                    .body(axum::body::Body::from("{\"tool\":\"click\"}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    #[serial]
    async fn enabled_but_tool_not_allowed() {
        let mut policy = AdapterPolicyView::default();
        policy.enabled = true;
        policy.tenants.push(TenantPolicy {
            id: "tenant-1".into(),
            allow_tools: vec!["navigate".into()],
            allow_flows: Vec::new(),
            read_only: Vec::new(),
            rate_limit_rps: 1,
            rate_burst: 1,
            concurrency_max: 1,
            timeout_ms_tool: 1_000,
            timeout_ms_flow: 5_000,
            timeout_ms_read: 2_000,
            idempotency_window_sec: 60,
            allow_cold_export: false,
            exports_max_lines: 10_000,
            authz_scopes: Vec::new(),
            api_keys: Vec::new(),
            hmac_secrets: Vec::new(),
        });
        set_policy(policy);

        let router = test_router();
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tools/run")
                    .header("content-type", "application/json")
                    .header("x-tenant-id", "tenant-1")
                    .body(axum::body::Body::from("{\"tool\":\"click\"}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    #[serial]
    async fn enabled_and_allowed_returns_not_implemented() {
        let mut policy = AdapterPolicyView::default();
        policy.enabled = true;
        policy.tenants.push(TenantPolicy {
            id: "tenant-1".into(),
            allow_tools: vec!["click".into()],
            allow_flows: Vec::new(),
            read_only: Vec::new(),
            rate_limit_rps: 1,
            rate_burst: 1,
            concurrency_max: 1,
            timeout_ms_tool: 1_000,
            timeout_ms_flow: 5_000,
            timeout_ms_read: 2_000,
            idempotency_window_sec: 60,
            allow_cold_export: false,
            exports_max_lines: 10_000,
            authz_scopes: Vec::new(),
            api_keys: Vec::new(),
            hmac_secrets: Vec::new(),
        });
        set_policy(policy);

        let router = test_router();
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tools/run")
                    .header("content-type", "application/json")
                    .header("x-tenant-id", "tenant-1")
                    .body(axum::body::Body::from("{\"tool\":\"click\"}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    #[serial]
    async fn adapter_requires_token_when_configured() {
        let mut policy = AdapterPolicyView::default();
        policy.enabled = true;
        policy.tenants.push(TenantPolicy {
            id: "tenant-1".into(),
            allow_tools: vec!["click".into()],
            allow_flows: Vec::new(),
            read_only: Vec::new(),
            rate_limit_rps: 1,
            rate_burst: 1,
            concurrency_max: 1,
            timeout_ms_tool: 1_000,
            timeout_ms_flow: 5_000,
            timeout_ms_read: 2_000,
            idempotency_window_sec: 60,
            allow_cold_export: false,
            exports_max_lines: 10_000,
            authz_scopes: Vec::new(),
            api_keys: vec!["secret".into()],
            hmac_secrets: Vec::new(),
        });
        set_policy(policy);

        let router = test_router();

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tools/run")
                    .header("content-type", "application/json")
                    .header("x-tenant-id", "tenant-1")
                    .body(axum::body::Body::from("{\"tool\":\"click\"}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tools/run")
                    .header("content-type", "application/json")
                    .header("x-tenant-id", "tenant-1")
                    .header("authorization", "Bearer secret")
                    .body(axum::body::Body::from("{\"tool\":\"click\"}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    #[serial]
    async fn adapter_accepts_hmac_signature() {
        let mut policy = AdapterPolicyView::default();
        policy.enabled = true;
        policy.tenants.push(TenantPolicy {
            id: "tenant-1".into(),
            allow_tools: vec!["click".into()],
            allow_flows: Vec::new(),
            read_only: Vec::new(),
            rate_limit_rps: 10,
            rate_burst: 5,
            concurrency_max: 2,
            timeout_ms_tool: 5_000,
            timeout_ms_flow: 5_000,
            timeout_ms_read: 2_000,
            idempotency_window_sec: 60,
            allow_cold_export: false,
            exports_max_lines: 10_000,
            authz_scopes: Vec::new(),
            api_keys: Vec::new(),
            hmac_secrets: vec!["shared".into()],
        });
        set_policy(policy);

        let router = test_router();

        let body = json!({ "tool": "click" }).to_string();
        let timestamp = OffsetDateTime::now_utc().unix_timestamp().to_string();
        let message = format!("{}:{}", timestamp, body);
        let mut mac = Hmac::<Sha256>::new_from_slice(b"shared").unwrap();
        mac.update(message.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tools/run")
                    .header("content-type", "application/json")
                    .header("x-tenant-id", "tenant-1")
                    .header("x-signature", signature)
                    .header("x-signature-timestamp", timestamp)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    #[serial]
    async fn adapter_uses_idempotency_cache() {
        let mut policy = AdapterPolicyView::default();
        policy.enabled = true;
        policy.tenants.push(TenantPolicy {
            id: "tenant-1".into(),
            allow_tools: vec!["click".into()],
            allow_flows: Vec::new(),
            read_only: Vec::new(),
            rate_limit_rps: 10,
            rate_burst: 5,
            concurrency_max: 2,
            timeout_ms_tool: 5_000,
            timeout_ms_flow: 5_000,
            timeout_ms_read: 2_000,
            idempotency_window_sec: 60,
            allow_cold_export: false,
            exports_max_lines: 10_000,
            authz_scopes: Vec::new(),
            api_keys: Vec::new(),
            hmac_secrets: Vec::new(),
        });
        set_policy(policy);

        let calls = Arc::new(AtomicUsize::new(0));

        struct CountingDispatcher {
            calls: Arc<AtomicUsize>,
        }

        #[async_trait::async_trait]
        impl DispatcherPort for CountingDispatcher {
            async fn run_tool(&self, _call: ToolCall) -> AdapterResult<ToolOutcome> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(ToolOutcome {
                    status: "ok".into(),
                    data: Some(serde_json::json!({"count": 1})),
                    trace_id: Some("trace-1".into()),
                    action_id: Some("action-1".into()),
                })
            }
        }

        let dispatcher: Arc<dyn DispatcherPort> = Arc::new(CountingDispatcher {
            calls: Arc::clone(&calls),
        });
        let router = router_with_dispatcher(dispatcher);

        let request = |key: &str| {
            Request::builder()
                .method("POST")
                .uri("/v1/tools/run")
                .header("content-type", "application/json")
                .header("x-tenant-id", "tenant-1")
                .body(axum::body::Body::from(
                    serde_json::json!({
                        "tool": "click",
                        "idempotency_key": key
                    })
                    .to_string(),
                ))
                .unwrap()
        };

        let first = router.clone().oneshot(request("cache-key")).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let second = router.oneshot(request("cache-key")).await.unwrap();
        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    fn test_router() -> Router {
        router_with_dispatcher(Arc::new(NoopDispatcher))
    }

    fn router_with_dispatcher(dispatcher: Arc<dyn DispatcherPort>) -> Router {
        let policy_handle = AdapterPolicyHandle::global();
        router(
            policy_handle,
            dispatcher,
            Arc::new(NoopReadonly),
            Arc::new(crate::events::NoopEvents),
            AdapterTracer::default(),
            Arc::new(IdempotencyStore::new()),
        )
    }
}
