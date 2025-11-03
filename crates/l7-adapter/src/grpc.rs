use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use time::OffsetDateTime;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use crate::auth;
use crate::errors::{AdapterError, AdapterResult};
use crate::events::{AdapterResponseEvent, EventsPort};
use crate::http::{self, AdapterState};
use crate::ports::ToolCall;
use crate::privacy;
use crate::trace::AdapterTracer;

pub mod proto {
    tonic::include_proto!("l7.adapter");
}

use proto::adapter_service_server::{AdapterService, AdapterServiceServer};
use proto::{RunToolRequest, RunToolResponse};

#[derive(Clone)]
pub(crate) struct GrpcAdapter {
    state: http::AdapterState,
}

impl GrpcAdapter {
    fn new(state: http::AdapterState) -> Self {
        Self { state }
    }

    fn events(&self) -> Arc<dyn EventsPort> {
        self.state.events()
    }

    fn tracer(&self) -> AdapterTracer {
        self.state.tracer()
    }
}

pub(crate) fn service(state: AdapterState) -> AdapterServiceServer<GrpcAdapter> {
    AdapterServiceServer::new(GrpcAdapter::new(state))
}

pub async fn serve(addr: SocketAddr, state: AdapterState) -> AdapterResult<()> {
    Server::builder()
        .add_service(service(state))
        .serve(addr)
        .await
        .map_err(|error| {
            tracing::error!(%error, "gRPC server error");
            AdapterError::Internal
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::AdapterBootstrap;
    use crate::events::NoopEvents;
    use crate::policy::{AdapterPolicyHandle, AdapterPolicyView, TenantPolicy};
    use crate::ports::{DispatcherPort, NoopReadonly, ToolOutcome};
    use serde_json::json;
    use serial_test::serial;
    use std::sync::Arc;
    use tokio::sync::oneshot;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::{Endpoint, Server};
    use tonic::Request;

    use super::proto::adapter_service_client::AdapterServiceClient;

    struct OkDispatcher;

    #[async_trait::async_trait]
    impl DispatcherPort for OkDispatcher {
        async fn run_tool(&self, call: ToolCall) -> crate::AdapterResult<ToolOutcome> {
            Ok(ToolOutcome {
                status: format!("ok:{}", call.tool),
                data: Some(json!({ "tenant": call.tenant_id })),
                trace_id: call.trace_id.clone(),
                action_id: Some("action-1".into()),
            })
        }
    }

    fn setup_policy() -> AdapterPolicyHandle {
        let handle = AdapterPolicyHandle::global();
        let mut view = AdapterPolicyView::default();
        view.enabled = true;
        view.tenants.push(TenantPolicy {
            id: "tenant-1".into(),
            allow_tools: vec!["click".into()],
            allow_flows: Vec::new(),
            read_only: Vec::new(),
            rate_limit_rps: 10,
            rate_burst: 10,
            concurrency_max: 5,
            timeout_ms_tool: 5_000,
            timeout_ms_flow: 5_000,
            timeout_ms_read: 5_000,
            idempotency_window_sec: 60,
            allow_cold_export: false,
            exports_max_lines: 10_000,
            authz_scopes: Vec::new(),
            api_keys: Vec::new(),
            hmac_secrets: Vec::new(),
        });
        handle.update(view);
        handle
    }

    #[tokio::test]
    #[serial]
    async fn run_tool_roundtrip() {
        let policy_handle = setup_policy();
        let bootstrap = AdapterBootstrap::new(
            policy_handle.clone(),
            Arc::new(OkDispatcher),
            Arc::new(NoopReadonly),
        )
        .with_events(Arc::new(NoopEvents));
        let state = bootstrap.state();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            Server::builder()
                .add_service(service(state))
                .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async move {
                    let _ = rx.await;
                })
                .await
                .unwrap();
        });

        let endpoint = Endpoint::from_shared(format!("http://{}", addr))
            .expect("endpoint")
            .connect()
            .await
            .expect("connect client");
        let mut client = AdapterServiceClient::new(endpoint);

        let request = RunToolRequest {
            tenant_id: "tenant-1".into(),
            tool: "click".into(),
            params_json: json!({}).to_string(),
            routing_json: json!({}).to_string(),
            options_json: json!({}).to_string(),
            timeout_ms: 1_000,
            idempotency_key: String::new(),
            trace_id: "trace-1".into(),
        };

        let response = client
            .run_tool(Request::new(request))
            .await
            .expect("grpc response")
            .into_inner();

        assert_eq!(response.status, "ok:click");
        assert!(response.data_json.contains("tenant-1"));

        let _ = tx.send(());
    }
}

fn parse_json(field: &str, input: &str) -> Result<Value, Status> {
    if input.is_empty() {
        Ok(Value::Null)
    } else {
        serde_json::from_str(input)
            .map_err(|err| Status::invalid_argument(format!("invalid {} json: {}", field, err)))
    }
}

fn adapter_error_to_status(err: AdapterError) -> Status {
    match err {
        AdapterError::Disabled => Status::unavailable("adapter disabled"),
        AdapterError::UnauthorizedTenant | AdapterError::TenantNotFound => {
            Status::permission_denied("tenant not permitted")
        }
        AdapterError::ToolNotAllowed => Status::permission_denied("tool not allowed"),
        AdapterError::TooManyRequests => Status::resource_exhausted("rate limited"),
        AdapterError::ConcurrencyLimit => Status::resource_exhausted("concurrency limited"),
        AdapterError::InvalidArgument => Status::invalid_argument("invalid argument"),
        AdapterError::NotImplemented(_) => Status::unimplemented("not implemented"),
        AdapterError::Internal => Status::internal("internal error"),
    }
}

fn policy_violation(message: &str) -> Status {
    Status::permission_denied(message)
}

#[tonic::async_trait]
impl AdapterService for GrpcAdapter {
    async fn run_tool(
        &self,
        request: Request<RunToolRequest>,
    ) -> Result<Response<RunToolResponse>, Status> {
        let metadata = request.metadata().clone();
        let inner = request.into_inner();
        let RunToolRequest {
            tenant_id,
            tool,
            params_json,
            routing_json,
            options_json,
            timeout_ms,
            idempotency_key,
            trace_id,
        } = inner;

        let payload_json = json!({
            "tenant_id": tenant_id.clone(),
            "tool": tool.clone(),
            "params_json": params_json.clone(),
            "routing_json": routing_json.clone(),
            "options_json": options_json.clone(),
            "timeout_ms": timeout_ms,
            "idempotency_key": idempotency_key.clone(),
            "trace_id": trace_id.clone(),
        })
        .to_string();

        let params = parse_json("params", &params_json)?;
        let routing = parse_json("routing", &routing_json)?;
        let options = parse_json("options", &options_json)?;

        let policy = self.state.snapshot();
        if !policy.enabled {
            return Err(adapter_error_to_status(AdapterError::Disabled));
        }

        let tenant = tenant_id.clone();
        let tenant_policy = policy
            .tenant(&tenant)
            .cloned()
            .ok_or_else(|| policy_violation("tenant not permitted"))?;

        auth::verify_grpc(&metadata, &tenant_policy, &payload_json)
            .map_err(|msg| Status::unauthenticated(msg))?;

        if !tenant_policy.allow_tools.is_empty()
            && !tenant_policy
                .allow_tools
                .iter()
                .any(|allowed| allowed == &tool)
        {
            return Err(policy_violation("tool not allowed"));
        }

        let timeout_ms = if timeout_ms == 0 {
            10_000
        } else {
            timeout_ms.min(tenant_policy.timeout_ms_tool.max(1))
        };

        let mut call = ToolCall {
            tenant_id: tenant.clone(),
            tool: tool.clone(),
            params,
            routing,
            options,
            timeout_ms,
            idempotency_key: if idempotency_key.is_empty() {
                None
            } else {
                Some(idempotency_key.clone())
            },
            trace_id: if trace_id.is_empty() {
                None
            } else {
                Some(trace_id.clone())
            },
        };

        privacy::sanitize_tool_call(&mut call);
        self.events().on_request(&call);

        let span = self.tracer().span(&call.tenant_id, &call.tool);
        let _enter = span.enter();

        let idempotency_ttl = if tenant_policy.idempotency_window_sec == 0 {
            None
        } else {
            Some(Duration::from_secs(tenant_policy.idempotency_window_sec))
        };

        if idempotency_ttl.is_some() {
            if let Some(key) = call.idempotency_key.as_ref() {
                if let Some(cached) = self.state.idempotency().lookup(&call.tenant_id, key) {
                    self.events().on_response(&call, &cached);
                    self.events().adapter_response(AdapterResponseEvent {
                        tenant_id: call.tenant_id.clone(),
                        tool: call.tool.clone(),
                        trace_id: cached.trace_id.clone(),
                        action_id: cached.action_id.clone(),
                        latency_ms: Some(0),
                        status: cached.status.clone(),
                        timestamp: Some(std::time::SystemTime::now()),
                    });

                    let data_json = cached
                        .data
                        .map(|value| serde_json::to_string(&value).unwrap_or_else(|_| "{}".into()))
                        .unwrap_or_else(|| "{}".into());

                    let response = RunToolResponse {
                        status: cached.status,
                        data_json,
                        trace_id: cached.trace_id.unwrap_or_default(),
                        action_id: cached.action_id.unwrap_or_default(),
                    };

                    return Ok(Response::new(response));
                }
            }
        }

        let permit = self
            .state
            .guard()
            .enter(&tenant_policy)
            .map_err(adapter_error_to_status)?;

        let started = OffsetDateTime::now_utc();
        let outcome = self
            .state
            .dispatcher()
            .run_tool(call.clone())
            .await
            .map_err(adapter_error_to_status)?;

        drop(permit);

        let mut outcome = outcome;
        privacy::sanitize_tool_outcome(&call, &mut outcome);

        self.events().on_response(&call, &outcome);
        self.events().adapter_response(AdapterResponseEvent {
            tenant_id: call.tenant_id.clone(),
            tool: call.tool.clone(),
            trace_id: outcome.trace_id.clone(),
            action_id: outcome.action_id.clone(),
            latency_ms: Some((OffsetDateTime::now_utc() - started).whole_milliseconds() as u128),
            status: outcome.status.clone(),
            timestamp: Some(std::time::SystemTime::now()),
        });

        if let Some(ttl) = idempotency_ttl {
            if let Some(key) = call.idempotency_key.as_ref() {
                self.state
                    .idempotency()
                    .insert(&call.tenant_id, key.clone(), ttl, &outcome);
            }
        }

        let data_json = outcome
            .data
            .map(|value| serde_json::to_string(&value).unwrap_or_else(|_| "{}".into()))
            .unwrap_or_else(|| "{}".into());

        let response = RunToolResponse {
            status: outcome.status,
            data_json,
            trace_id: outcome.trace_id.unwrap_or_default(),
            action_id: outcome.action_id.unwrap_or_default(),
        };

        Ok(Response::new(response))
    }
}
