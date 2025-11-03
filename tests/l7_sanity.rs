use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use l7_adapter::events::NoopEvents;
use l7_adapter::idempotency::IdempotencyStore;
use l7_adapter::ports::{DispatcherPort, NoopReadonly, ToolCall, ToolOutcome};
use l7_adapter::{router, AdapterPolicyHandle, AdapterPolicyView, TenantPolicy};
use parking_lot::Mutex;
use serde_json::json;
use serial_test::serial;
use std::sync::Arc;
use tokio::sync::Notify;
use tokio::time::{sleep, Duration};
use tower::ServiceExt;

struct OkDispatcher;

#[async_trait::async_trait]
impl DispatcherPort for OkDispatcher {
    async fn run_tool(&self, call: ToolCall) -> l7_adapter::AdapterResult<ToolOutcome> {
        Ok(ToolOutcome {
            status: format!("ok:{}", call.tool),
            data: Some(json!({ "tenant": call.tenant_id })),
            ..ToolOutcome::default()
        })
    }
}

static POLICY_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test]
#[serial]
async fn adapter_accepts_authorized_tool() {
    let _guard = POLICY_LOCK.lock();

    let policy_handle = AdapterPolicyHandle::global();
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
    policy_handle.update(policy);

    let router = build_router(policy_handle, Arc::new(OkDispatcher));
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tools/run")
                .header("content-type", "application/json")
                .header("x-tenant-id", "tenant-1")
                .body(Body::from(json!({ "tool": "click" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["status"], "ok:click");
    assert_eq!(body["data"]["tenant"], "tenant-1");
}

struct BlockingDispatcher {
    notify: Arc<Notify>,
}

#[async_trait::async_trait]
impl DispatcherPort for BlockingDispatcher {
    async fn run_tool(&self, _call: ToolCall) -> l7_adapter::AdapterResult<ToolOutcome> {
        self.notify.notified().await;
        Ok(ToolOutcome {
            status: "ok:block".into(),
            data: None,
            ..ToolOutcome::default()
        })
    }
}

#[tokio::test]
#[serial]
async fn adapter_enforces_concurrency_limit() {
    let _guard = POLICY_LOCK.lock();

    let policy_handle = AdapterPolicyHandle::global();
    let mut policy = AdapterPolicyView::default();
    policy.enabled = true;
    policy.tenants.push(TenantPolicy {
        id: "tenant-1".into(),
        allow_tools: vec!["click".into()],
        allow_flows: Vec::new(),
        read_only: Vec::new(),
        rate_limit_rps: 0,
        rate_burst: 0,
        concurrency_max: 1,
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
    policy_handle.update(policy);

    let notify = Arc::new(Notify::new());
    let dispatcher = Arc::new(BlockingDispatcher {
        notify: notify.clone(),
    });
    let router = build_router(policy_handle, dispatcher);

    let request = || {
        Request::builder()
            .method("POST")
            .uri("/v1/tools/run")
            .header("content-type", "application/json")
            .header("x-tenant-id", "tenant-1")
            .body(Body::from(json!({ "tool": "click" }).to_string()))
            .unwrap()
    };

    let first_router = router.clone();
    let first_handle = tokio::spawn(async move { first_router.oneshot(request()).await.unwrap() });

    sleep(Duration::from_millis(50)).await;

    let second_response = router.clone().oneshot(request()).await.unwrap();
    assert_eq!(second_response.status(), StatusCode::TOO_MANY_REQUESTS);

    notify.notify_one();
    let first_response = first_handle.await.unwrap();
    assert_eq!(first_response.status(), StatusCode::OK);
}

fn build_router(policy_handle: AdapterPolicyHandle, dispatcher: Arc<dyn DispatcherPort>) -> Router {
    router(
        policy_handle,
        dispatcher,
        Arc::new(NoopReadonly),
        Arc::new(NoopEvents),
        l7_adapter::trace::AdapterTracer::default(),
        Arc::new(IdempotencyStore::new()),
    )
}
