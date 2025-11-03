use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{response::IntoResponse, Json};
use l6_privacy::apply_obs;
use l6_privacy::context::RedactCtx;
use l6_privacy::RedactScope;
use serde_json::{json, Value};
use std::sync::Arc;
use url::Url;

const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

use crate::auth;
use crate::dispatcher::ToolDispatcher;
use crate::errors::{BridgeError, BridgeResult};
use crate::mapping;
use crate::model::{FindElementRequest, NavigateToUrlRequest, NewSessionRequest};
use crate::policy::{TenantPolicy, WebDriverBridgePolicyHandle};
use crate::state::SessionStore;
use crate::trace::BridgeTracer;

#[derive(Clone)]
pub struct BridgeCtx {
    pub policy: WebDriverBridgePolicyHandle,
    pub state: Arc<SessionStore>,
    pub tracer: BridgeTracer,
    pub dispatcher: Arc<dyn ToolDispatcher>,
}

pub async fn status() -> impl IntoResponse {
    Json(json!({
        "value": {
            "ready": true,
            "message": "webdriver bridge skeleton"
        }
    }))
}

pub async fn create_session(
    State(ctx): State<BridgeCtx>,
    headers: HeaderMap,
    Json(req): Json<NewSessionRequest>,
) -> BridgeResult<Json<Value>> {
    let policy = ctx.policy.snapshot();
    let tenant = auth::authenticate(&headers, &policy)?;
    let tenant_policy = ensure_tenant(&policy, &tenant)?;
    ensure_endpoint(&tenant_policy, "session.create")?;

    let capabilities = req
        .capabilities
        .unwrap_or_else(|| default_capabilities(&policy));
    let session = ctx.state.create(tenant.clone(), capabilities.clone());

    let payload = json!({
        "value": {
            "sessionId": session.session_id,
            "capabilities": capabilities,
        }
    });

    Ok(Json(payload))
}

pub async fn delete_session(
    State(ctx): State<BridgeCtx>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> BridgeResult<StatusCode> {
    let policy = ctx.policy.snapshot();
    let tenant = auth::authenticate(&headers, &policy)?;
    let tenant_policy = ensure_tenant(&policy, &tenant)?;
    ensure_endpoint(&tenant_policy, "session.delete")?;

    match ctx.state.get(&session_id) {
        Some(session) if session.tenant_id == tenant => {
            ctx.state.remove(&session_id);
            Ok(StatusCode::NO_CONTENT)
        }
        Some(_) => Err(BridgeError::Forbidden),
        None => Err(BridgeError::NoSuchSession),
    }
}

pub async fn navigate_to_url(
    State(ctx): State<BridgeCtx>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(req): Json<NavigateToUrlRequest>,
) -> BridgeResult<Json<Value>> {
    let policy = ctx.policy.snapshot();
    let tenant = auth::authenticate(&headers, &policy)?;
    let tenant_policy = ensure_tenant(&policy, &tenant)?;
    ensure_endpoint(&tenant_policy, "session.url.post")?;
    ensure_tool(&tenant_policy, "navigate-to-url")?;

    let session = ctx
        .state
        .get(&session_id)
        .ok_or(BridgeError::NoSuchSession)?;
    if session.tenant_id != tenant {
        return Err(BridgeError::Forbidden);
    }

    if req.url.trim().is_empty() {
        return Err(BridgeError::InvalidArgument);
    }
    let parsed = Url::parse(&req.url).map_err(|_| BridgeError::InvalidArgument)?;

    let span = ctx.tracer.span("navigate_to_url");
    let _guard = span.enter();
    let payload = json!({ "url": parsed.as_str() });
    let tool_call = mapping::to_tool_call("navigateTo", payload)?;
    ctx.dispatcher.run_tool(&tenant, tool_call, None).await?;

    ctx.state
        .set_current_url(&session_id, parsed.as_str().to_string());

    Ok(Json(json!({ "value": null })))
}

pub async fn find_element(
    State(ctx): State<BridgeCtx>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(req): Json<FindElementRequest>,
) -> BridgeResult<Json<Value>> {
    let policy = ctx.policy.snapshot();
    let tenant = auth::authenticate(&headers, &policy)?;
    let tenant_policy = ensure_tenant(&policy, &tenant)?;
    ensure_endpoint(&tenant_policy, "element.find")?;

    let session = ctx
        .state
        .get(&session_id)
        .ok_or(BridgeError::NoSuchSession)?;
    if session.tenant_id != tenant {
        return Err(BridgeError::Forbidden);
    }

    validate_selector(&policy, &req)?;

    let element_id = ctx
        .state
        .allocate_element(&session_id, &req.using, &req.value)
        .ok_or(BridgeError::Internal)?;

    Ok(Json(json!({
        "value": {
            ELEMENT_KEY: element_id,
        }
    })))
}

pub async fn find_elements(
    State(ctx): State<BridgeCtx>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(req): Json<FindElementRequest>,
) -> BridgeResult<Json<Value>> {
    let policy = ctx.policy.snapshot();
    let tenant = auth::authenticate(&headers, &policy)?;
    let tenant_policy = ensure_tenant(&policy, &tenant)?;
    ensure_endpoint(&tenant_policy, "element.find_many")?;

    let session = ctx
        .state
        .get(&session_id)
        .ok_or(BridgeError::NoSuchSession)?;
    if session.tenant_id != tenant {
        return Err(BridgeError::Forbidden);
    }

    validate_selector(&policy, &req)?;

    let element_id = ctx
        .state
        .allocate_element(&session_id, &req.using, &req.value)
        .ok_or(BridgeError::Internal)?;

    Ok(Json(json!({
        "value": [
            {
                ELEMENT_KEY: element_id,
            }
        ]
    })))
}

pub async fn current_url(
    State(ctx): State<BridgeCtx>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> BridgeResult<Json<Value>> {
    let policy = ctx.policy.snapshot();
    let tenant = auth::authenticate(&headers, &policy)?;
    let tenant_policy = ensure_tenant(&policy, &tenant)?;
    ensure_endpoint(&tenant_policy, "session.url.get")?;

    let session = ctx
        .state
        .get(&session_id)
        .ok_or(BridgeError::NoSuchSession)?;
    if session.tenant_id != tenant {
        return Err(BridgeError::Forbidden);
    }

    let raw_url = session.current_url.clone().unwrap_or_default();
    let sanitized = Url::parse(&raw_url)
        .ok()
        .map(|url| format!("{}{}", url.host_str().unwrap_or_default(), url.path()))
        .unwrap_or(raw_url);

    Ok(Json(json!({ "value": sanitized })))
}

pub async fn click_element(
    State(ctx): State<BridgeCtx>,
    headers: HeaderMap,
    Path((session_id, element_id)): Path<(String, String)>,
) -> BridgeResult<Json<Value>> {
    let policy = ctx.policy.snapshot();
    let tenant = auth::authenticate(&headers, &policy)?;
    let tenant_policy = ensure_tenant(&policy, &tenant)?;
    ensure_endpoint(&tenant_policy, "element.click")?;
    ensure_tool(&tenant_policy, "click")?;

    let session = ctx
        .state
        .get(&session_id)
        .ok_or(BridgeError::NoSuchSession)?;
    if session.tenant_id != tenant {
        return Err(BridgeError::Forbidden);
    }

    match ctx.state.element_entry(&element_id) {
        Some(entry) if entry.session_id == session_id => {}
        Some(_) => return Err(BridgeError::Forbidden),
        None => return Err(BridgeError::NoSuchElement),
    }

    let payload = json!({
        "element": element_id,
    });
    let tool_call = mapping::to_tool_call("clickElement", payload)?;
    ctx.dispatcher.run_tool(&tenant, tool_call, None).await?;

    Ok(Json(json!({ "value": null })))
}

pub async fn element_text(
    State(ctx): State<BridgeCtx>,
    headers: HeaderMap,
    Path((session_id, element_id)): Path<(String, String)>,
) -> BridgeResult<Json<Value>> {
    let policy = ctx.policy.snapshot();
    let tenant = auth::authenticate(&headers, &policy)?;
    let tenant_policy = ensure_tenant(&policy, &tenant)?;
    ensure_endpoint(&tenant_policy, "element.text.get")?;
    ensure_tool(&tenant_policy, "get-text")?;

    let session = ctx
        .state
        .get(&session_id)
        .ok_or(BridgeError::NoSuchSession)?;
    if session.tenant_id != tenant {
        return Err(BridgeError::Forbidden);
    }

    let entry = match ctx.state.element_entry(&element_id) {
        Some(entry) if entry.session_id == session_id => entry,
        Some(_) => return Err(BridgeError::Forbidden),
        None => return Err(BridgeError::NoSuchElement),
    };

    let payload = json!({ "element": element_id });
    let tool_call = mapping::to_tool_call("getElementText", payload)?;
    let result = ctx.dispatcher.run_tool(&tenant, tool_call, None).await?;

    let text = result
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(entry.last_seen_text)
        .unwrap_or_default();
    ctx.state.update_element_text(&element_id, &text);

    let mut json_text = json!({ "text": text });
    let redact_ctx = RedactCtx {
        scope: RedactScope::Observation,
        ..Default::default()
    };
    let _ = apply_obs(&mut json_text, &redact_ctx);
    let value = json_text.get("text").cloned().unwrap_or_else(|| json!(""));

    Ok(Json(json!({ "value": value })))
}

pub async fn element_attribute(
    State(ctx): State<BridgeCtx>,
    headers: HeaderMap,
    Path((session_id, element_id, attr)): Path<(String, String, String)>,
) -> BridgeResult<Json<Value>> {
    let policy = ctx.policy.snapshot();
    let tenant = auth::authenticate(&headers, &policy)?;
    let tenant_policy = ensure_tenant(&policy, &tenant)?;
    ensure_endpoint(&tenant_policy, "element.attribute.get")?;
    ensure_tool(&tenant_policy, "get-attribute")?;

    let session = ctx
        .state
        .get(&session_id)
        .ok_or(BridgeError::NoSuchSession)?;
    if session.tenant_id != tenant {
        return Err(BridgeError::Forbidden);
    }

    let entry = match ctx.state.element_entry(&element_id) {
        Some(entry) if entry.session_id == session_id => entry,
        Some(_) => return Err(BridgeError::Forbidden),
        None => return Err(BridgeError::NoSuchElement),
    };

    let payload = json!({
        "element": element_id,
        "attribute": attr,
    });
    let tool_call = mapping::to_tool_call("getElementAttribute", payload)?;
    let result = ctx.dispatcher.run_tool(&tenant, tool_call, None).await?;

    let value = result
        .get("value")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(entry.attributes.get(&attr).cloned())
        .unwrap_or_default();
    ctx.state
        .update_element_attribute(&element_id, &attr, &value);

    let mut json_attr = json!({ "value": value });
    let redact_ctx = RedactCtx {
        scope: RedactScope::Observation,
        ..Default::default()
    };
    let _ = apply_obs(&mut json_attr, &redact_ctx);
    let value = json_attr.get("value").cloned().unwrap_or_else(|| json!(""));

    Ok(Json(json!({ "value": value })))
}

pub async fn get_title(
    State(ctx): State<BridgeCtx>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> BridgeResult<Json<Value>> {
    let policy = ctx.policy.snapshot();
    let tenant = auth::authenticate(&headers, &policy)?;
    let tenant_policy = ensure_tenant(&policy, &tenant)?;
    ensure_endpoint(&tenant_policy, "session.title.get")?;
    ensure_tool(&tenant_policy, "get-title")?;

    let session = ctx
        .state
        .get(&session_id)
        .ok_or(BridgeError::NoSuchSession)?;
    if session.tenant_id != tenant {
        return Err(BridgeError::Forbidden);
    }

    let payload = json!({});
    let tool_call = mapping::to_tool_call("getTitle", payload)?;
    let result = ctx.dispatcher.run_tool(&tenant, tool_call, None).await?;

    let title = result
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let mut json_title = json!({ "title": title });
    let redact_ctx = RedactCtx {
        scope: RedactScope::Observation,
        ..Default::default()
    };
    let _ = apply_obs(&mut json_title, &redact_ctx);
    let value = json_title
        .get("title")
        .cloned()
        .unwrap_or_else(|| json!(""));

    Ok(Json(json!({ "value": value })))
}

fn ensure_tenant<'a>(
    policy: &'a crate::policy::WebDriverBridgePolicy,
    tenant_id: &str,
) -> BridgeResult<TenantPolicy> {
    policy
        .tenants
        .iter()
        .find(|tenant| tenant.id == tenant_id && tenant.enable)
        .cloned()
        .ok_or(BridgeError::Forbidden)
}

fn ensure_endpoint(tenant: &TenantPolicy, endpoint: &str) -> BridgeResult<()> {
    if tenant.allow_endpoints.is_empty() || tenant.allow_endpoints.iter().any(|e| e == endpoint) {
        Ok(())
    } else {
        Err(BridgeError::Forbidden)
    }
}

fn ensure_tool(tenant: &TenantPolicy, tool: &str) -> BridgeResult<()> {
    if tenant.allow_tools.is_empty() || tenant.allow_tools.iter().any(|t| t == tool) {
        Ok(())
    } else {
        Err(BridgeError::Forbidden)
    }
}

fn validate_selector(
    policy: &crate::policy::WebDriverBridgePolicy,
    req: &FindElementRequest,
) -> BridgeResult<()> {
    if req.value.trim().is_empty() {
        return Err(BridgeError::InvalidArgument);
    }

    match req.using.as_str() {
        "css selector" => Ok(()),
        "xpath" | "xpath selector" => {
            if policy.allow_xpath {
                Ok(())
            } else {
                Err(BridgeError::InvalidArgument)
            }
        }
        _ => Err(BridgeError::InvalidArgument),
    }
}

fn default_capabilities(policy: &crate::policy::WebDriverBridgePolicy) -> Value {
    json!({
        "browserName": "soulbrowser",
        "soul:privacyProfile": policy
            .privacy_profile
            .clone()
            .unwrap_or_else(|| "strict".into()),
        "acceptInsecureCerts": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use axum::Router;
    use serde_json::{json, Value as JsonValue};
    use serial_test::serial;
    use tower::ServiceExt;

    use crate::bootstrap::WebDriverBridge;
    use crate::policy::{TenantPolicy, WebDriverBridgePolicy};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct RecordingDispatcher {
        calls: Arc<Mutex<Vec<JsonValue>>>,
        response: Arc<Mutex<JsonValue>>,
    }

    impl RecordingDispatcher {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                response: Arc::new(Mutex::new(json!({}))),
            }
        }

        fn set_response(&self, value: JsonValue) {
            *self.response.lock().unwrap() = value;
        }
    }

    impl Default for RecordingDispatcher {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl ToolDispatcher for RecordingDispatcher {
        async fn run_tool(
            &self,
            _tenant: &str,
            call: soulbrowser_core_types::ToolCall,
            _routing: Option<soulbrowser_core_types::RoutingHint>,
        ) -> BridgeResult<JsonValue> {
            self.calls.lock().unwrap().push(json!({
                "tool": call.tool,
                "payload": call.payload,
            }));
            Ok(self.response.lock().unwrap().clone())
        }
    }

    fn setup_policy() -> WebDriverBridgePolicyHandle {
        let handle = WebDriverBridgePolicyHandle::global();
        let mut view = WebDriverBridgePolicy::default();
        view.enabled = true;
        view.tenants.push(TenantPolicy {
            id: "tenant-a".into(),
            enable: true,
            allow_endpoints: vec![],
            allow_tools: vec![],
            origins_allow: vec![],
            concurrency_max: 4,
        });
        handle.update(view);
        handle
    }

    fn router() -> (Router, Arc<RecordingDispatcher>) {
        let handle = setup_policy();
        let dispatcher = Arc::new(RecordingDispatcher::new());
        dispatcher.set_response(json!({}));
        let dispatcher_dyn: Arc<dyn ToolDispatcher> = dispatcher.clone();
        let router = WebDriverBridge::new(handle)
            .with_dispatcher(dispatcher_dyn)
            .build();
        (router, dispatcher)
    }

    fn session_request() -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/session")
            .header("content-type", "application/json")
            .header("x-tenant-id", "tenant-a")
            .body(Body::from("{}"))
            .unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn create_session_success() {
        let (router, _) = router();
        let response = router.clone().oneshot(session_request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        assert!(json["value"]["sessionId"].as_str().is_some());
        assert_eq!(
            json["value"]["capabilities"]["browserName"].as_str(),
            Some("soulbrowser")
        );
    }

    #[tokio::test]
    #[serial]
    async fn delete_session_removes_entry() {
        let (router, _) = router();
        let create_response = router.clone().oneshot(session_request()).await.unwrap();
        let body = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let session_id = json["value"]["sessionId"].as_str().unwrap();

        let delete_req = Request::builder()
            .method("DELETE")
            .uri(format!("/session/{}", session_id))
            .header("x-tenant-id", "tenant-a")
            .body(Body::empty())
            .unwrap();

        let response = router.clone().oneshot(delete_req).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let delete_again = Request::builder()
            .method("DELETE")
            .uri(format!("/session/{}", session_id))
            .header("x-tenant-id", "tenant-a")
            .body(Body::empty())
            .unwrap();

        let response = router.clone().oneshot(delete_again).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[serial]
    async fn missing_tenant_header_is_unauthorized() {
        let (router, _) = router();
        let request = Request::builder()
            .method("POST")
            .uri("/session")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();

        let response = router.clone().oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[serial]
    async fn get_title_returns_value() {
        let (router, dispatcher) = router();
        dispatcher.set_response(json!({}));
        let create_response = router.clone().oneshot(session_request()).await.unwrap();
        let body = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let session_id = json["value"]["sessionId"].as_str().unwrap();

        dispatcher.set_response(json!({ "title": "Secret Title" }));
        let req = Request::builder()
            .method("GET")
            .uri(format!("/session/{}/title", session_id))
            .header("x-tenant-id", "tenant-a")
            .body(Body::empty())
            .unwrap();

        let response = router.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        assert!(json["value"].is_object() || json["value"].as_str().is_some());
    }

    #[tokio::test]
    #[serial]
    async fn navigate_to_url_dispatches_tool() {
        let (router, dispatcher) = router();
        let create_response = router.clone().oneshot(session_request()).await.unwrap();
        let body = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let session_id = json["value"]["sessionId"].as_str().unwrap();

        let navigate_req = Request::builder()
            .method("POST")
            .uri(format!("/session/{}/url", session_id))
            .header("content-type", "application/json")
            .header("x-tenant-id", "tenant-a")
            .body(Body::from(
                json!({ "url": "https://example.com" }).to_string(),
            ))
            .unwrap();

        let response = router.clone().oneshot(navigate_req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let calls = dispatcher.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["tool"].as_str(), Some("navigate-to-url"));
        assert_eq!(
            calls[0]["payload"]["url"].as_str(),
            Some("https://example.com/")
        );

        let get_req = Request::builder()
            .method("GET")
            .uri(format!("/session/{}/url", session_id))
            .header("x-tenant-id", "tenant-a")
            .body(Body::empty())
            .unwrap();

        let response = router.clone().oneshot(get_req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["value"].as_str(), Some("example.com/"));
    }

    #[tokio::test]
    #[serial]
    async fn find_element_allocates_id() {
        let (router, _) = router();
        let create_response = router.clone().oneshot(session_request()).await.unwrap();
        let body = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let session_id = json["value"]["sessionId"].as_str().unwrap();

        let find_req = Request::builder()
            .method("POST")
            .uri(format!("/session/{}/element", session_id))
            .header("content-type", "application/json")
            .header("x-tenant-id", "tenant-a")
            .body(Body::from(
                json!({
                    "using": "css selector",
                    "value": "button.submit"
                })
                .to_string(),
            ))
            .unwrap();

        let response = router.clone().oneshot(find_req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let element_id = json["value"][ELEMENT_KEY].as_str().unwrap();
        assert!(element_id.starts_with("element-"));
    }

    #[tokio::test]
    #[serial]
    async fn find_elements_returns_array() {
        let (router, _) = router();
        let create_response = router.clone().oneshot(session_request()).await.unwrap();
        let body = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let session_id = json["value"]["sessionId"].as_str().unwrap();

        let find_req = Request::builder()
            .method("POST")
            .uri(format!("/session/{}/elements", session_id))
            .header("content-type", "application/json")
            .header("x-tenant-id", "tenant-a")
            .body(Body::from(
                json!({
                    "using": "css selector",
                    "value": "div.card"
                })
                .to_string(),
            ))
            .unwrap();

        let response = router.clone().oneshot(find_req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let array = json["value"].as_array().unwrap();
        assert_eq!(array.len(), 1);
        assert!(array[0][ELEMENT_KEY].is_string());
    }

    #[tokio::test]
    #[serial]
    async fn click_element_dispatches_tool() {
        let (router, dispatcher) = router();
        dispatcher.set_response(json!({}));
        let create_response = router.clone().oneshot(session_request()).await.unwrap();
        let body = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let session_id = json["value"]["sessionId"].as_str().unwrap();

        let find_req = Request::builder()
            .method("POST")
            .uri(format!("/session/{}/element", session_id))
            .header("content-type", "application/json")
            .header("x-tenant-id", "tenant-a")
            .body(Body::from(
                json!({
                    "using": "css selector",
                    "value": "button.submit"
                })
                .to_string(),
            ))
            .unwrap();

        let response = router.clone().oneshot(find_req).await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let element_id = json["value"][ELEMENT_KEY].as_str().unwrap().to_string();

        let click_req = Request::builder()
            .method("POST")
            .uri(format!(
                "/session/{}/element/{}/click",
                session_id, element_id
            ))
            .header("content-type", "application/json")
            .header("x-tenant-id", "tenant-a")
            .body(Body::from("{}"))
            .unwrap();

        let response = router.clone().oneshot(click_req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let calls = dispatcher.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["tool"].as_str(), Some("click"));
    }

    #[tokio::test]
    #[serial]
    async fn element_text_returns_value() {
        let (router, dispatcher) = router();
        dispatcher.set_response(json!({ "text": "hello" }));
        let create_response = router.clone().oneshot(session_request()).await.unwrap();
        let body = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let session_id = json["value"]["sessionId"].as_str().unwrap();

        let find_req = Request::builder()
            .method("POST")
            .uri(format!("/session/{}/element", session_id))
            .header("content-type", "application/json")
            .header("x-tenant-id", "tenant-a")
            .body(Body::from(
                json!({
                    "using": "css selector",
                    "value": "div.title"
                })
                .to_string(),
            ))
            .unwrap();

        let response = router.clone().oneshot(find_req).await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let element_id = json["value"][ELEMENT_KEY].as_str().unwrap();

        let text_req = Request::builder()
            .method("GET")
            .uri(format!(
                "/session/{}/element/{}/text",
                session_id, element_id
            ))
            .header("x-tenant-id", "tenant-a")
            .body(Body::empty())
            .unwrap();

        let response = router.clone().oneshot(text_req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        match json["value"].as_str() {
            Some(text) => assert_eq!(text, "hello"),
            None => assert!(json["value"]["len"].as_u64().is_some()),
        }
    }

    #[tokio::test]
    #[serial]
    async fn element_attribute_returns_value() {
        let (router, dispatcher) = router();
        dispatcher.set_response(json!({ "value": "https://example.com" }));
        let create_response = router.clone().oneshot(session_request()).await.unwrap();
        let body = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let session_id = json["value"]["sessionId"].as_str().unwrap();

        let find_req = Request::builder()
            .method("POST")
            .uri(format!("/session/{}/element", session_id))
            .header("content-type", "application/json")
            .header("x-tenant-id", "tenant-a")
            .body(Body::from(
                json!({
                    "using": "css selector",
                    "value": "a.link"
                })
                .to_string(),
            ))
            .unwrap();

        let response = router.clone().oneshot(find_req).await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        let element_id = json["value"][ELEMENT_KEY].as_str().unwrap();

        dispatcher.set_response(json!({ "value": "https://example.com" }));

        let attr_req = Request::builder()
            .method("GET")
            .uri(format!(
                "/session/{}/element/{}/attribute/href",
                session_id, element_id
            ))
            .header("x-tenant-id", "tenant-a")
            .body(Body::empty())
            .unwrap();

        let response = router.clone().oneshot(attr_req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        match json["value"].as_str() {
            Some(value) => assert_eq!(value, "https://example.com"),
            None => assert!(json["value"]["len"].as_u64().is_some()),
        }
    }
}
