//! Request/response interceptor module
#![allow(dead_code)]
//!
//! Provides request/response interception using soulbase-interceptors

use soulbase_interceptors::{
    context::InterceptContext,
    errors::InterceptError,
    policy::dsl::RoutePolicy,
    stages::{
        context_init::ContextInitStage, error_norm::ErrorNormStage, resilience::ResilienceStage,
        route_policy::RoutePolicyStage, schema_guard::SchemaGuardStage,
        tenant_guard::TenantGuardStage, Stage, StageOutcome,
    },
    InterceptorChain,
};

// Re-export ProtoRequest and ProtoResponse for public use
use crate::policy::merge_attrs;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use soulbase_auth::AuthFacade;
use soulbase_errors::prelude::codes;
pub use soulbase_interceptors::context::{ProtoRequest, ProtoResponse};
use soulbase_types::{subject::Subject, tenant::TenantId, trace::TraceContext};
use std::{sync::Arc, time::Duration};

/// Browser interceptor context
pub struct BrowserInterceptContext {
    pub inner: InterceptContext,
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

impl BrowserInterceptContext {
    /// Create new context
    pub fn new(_tenant: TenantId, subject: Subject) -> Self {
        // Create base intercept context

        let inner = InterceptContext {
            request_id: uuid::Uuid::new_v4().to_string(),
            trace: TraceContext {
                trace_id: Some(uuid::Uuid::new_v4().to_string()),
                span_id: Some(uuid::Uuid::new_v4().to_string()),
                baggage: Default::default(),
            },
            tenant_header: None,
            consent_token: None,
            route: None,
            subject: Some(subject),
            obligations: vec![],
            envelope_seed: Default::default(),
            authn_input: None,
            config_version: None,
            config_checksum: None,
            extensions: Default::default(),
            resilience: Default::default(),
        };

        Self {
            inner,
            metadata: serde_json::Map::new(),
        }
    }

    /// Add metadata
    pub fn add_metadata(&mut self, key: String, value: serde_json::Value) {
        self.metadata.insert(key, value);
    }

    /// Get metadata
    pub fn get_metadata(&self, key: &str) -> Option<&serde_json::Value> {
        self.metadata.get(key)
    }
}

/// Browser request wrapper for interceptors
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserRequest {
    pub method: String,
    pub path: String,
    pub headers: std::collections::HashMap<String, String>,
    pub body: Option<serde_json::Value>,
}

#[async_trait]
impl ProtoRequest for BrowserRequest {
    fn method(&self) -> &str {
        &self.method
    }

    fn path(&self) -> &str {
        &self.path
    }

    fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name).cloned()
    }

    async fn read_json(&mut self) -> Result<serde_json::Value, InterceptError> {
        Ok(self.body.clone().unwrap_or(serde_json::Value::Null))
    }
}

/// Browser response wrapper for interceptors
#[derive(Clone, Debug)]
pub struct BrowserResponse {
    pub status: u16,
    pub headers: std::collections::HashMap<String, String>,
    pub body: Option<serde_json::Value>,
}

impl BrowserResponse {
    pub fn new() -> Self {
        Self {
            status: 200,
            headers: std::collections::HashMap::new(),
            body: None,
        }
    }
}

#[async_trait]
impl ProtoResponse for BrowserResponse {
    fn set_status(&mut self, code: u16) {
        self.status = code;
    }

    fn insert_header(&mut self, name: &str, value: &str) {
        self.headers.insert(name.to_string(), value.to_string());
    }

    async fn write_json(&mut self, body: &serde_json::Value) -> Result<(), InterceptError> {
        self.body = Some(body.clone());
        self.insert_header("content-type", "application/json");
        Ok(())
    }
}

/// Logging interceptor stage
pub struct LoggingStage {
    level: LogLevel,
}

#[derive(Debug, Clone)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LoggingStage {
    pub fn new(level: LogLevel) -> Self {
        Self { level }
    }
}

#[async_trait]
impl Stage for LoggingStage {
    async fn handle(
        &self,
        cx: &mut InterceptContext,
        req: &mut dyn ProtoRequest,
        _rsp: &mut dyn ProtoResponse,
    ) -> Result<StageOutcome, InterceptError> {
        let method = req.method();
        let path = req.path();
        let tenant = cx.tenant_header.as_deref().unwrap_or("unknown");
        let subject = cx
            .subject
            .as_ref()
            .map(|s| s.subject_id.0.as_str())
            .unwrap_or("anonymous");
        let request_id = cx.request_id.as_str();
        let trace_id = cx.trace.trace_id.as_deref().unwrap_or("-");

        let message = "browser interceptor request";

        match self.level {
            LogLevel::Debug => {
                tracing::debug!(method, path, tenant, subject, request_id, trace_id, message)
            }
            LogLevel::Info => {
                tracing::info!(method, path, tenant, subject, request_id, trace_id, message)
            }
            LogLevel::Warn => {
                tracing::warn!(method, path, tenant, subject, request_id, trace_id, message)
            }
            LogLevel::Error => {
                tracing::error!(method, path, tenant, subject, request_id, trace_id, message)
            }
        }

        Ok(StageOutcome::Continue)
    }
}

/// Validation interceptor stage
pub struct ValidationStage {
    rules: Vec<ValidationRule>,
}

pub struct ValidationRule {
    pub field: String,
    pub required: bool,
    pub validator: Box<dyn Fn(&serde_json::Value) -> bool + Send + Sync>,
}

impl ValidationStage {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn add_rule(mut self, rule: ValidationRule) -> Self {
        self.rules.push(rule);
        self
    }
}

#[async_trait]
impl Stage for ValidationStage {
    async fn handle(
        &self,
        _cx: &mut InterceptContext,
        req: &mut dyn ProtoRequest,
        _rsp: &mut dyn ProtoResponse,
    ) -> Result<StageOutcome, InterceptError> {
        let body = req.read_json().await?;

        for rule in &self.rules {
            if let Some(value) = body.get(&rule.field) {
                if !(rule.validator)(value) {
                    return Err(InterceptError::schema(&format!(
                        "Validation failed for field: {}",
                        rule.field
                    )));
                }
            } else if rule.required {
                return Err(InterceptError::schema(&format!(
                    "Required field missing: {}",
                    rule.field
                )));
            }
        }

        Ok(StageOutcome::Continue)
    }
}

/// Rate limiting interceptor stage
pub struct RateLimitStage {
    max_requests: usize,
    window_seconds: u64,
    requests: Arc<tokio::sync::Mutex<std::collections::HashMap<String, Vec<std::time::Instant>>>>,
}

impl RateLimitStage {
    pub fn new(max_requests: usize, window_seconds: u64) -> Self {
        Self {
            max_requests,
            window_seconds,
            requests: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }
}

#[async_trait]
impl Stage for RateLimitStage {
    async fn handle(
        &self,
        cx: &mut InterceptContext,
        _req: &mut dyn ProtoRequest,
        _rsp: &mut dyn ProtoResponse,
    ) -> Result<StageOutcome, InterceptError> {
        let subject = cx
            .subject
            .as_ref()
            .ok_or_else(|| InterceptError::internal("No subject in context"))?;
        let subject_id = &subject.subject_id.0;
        let now = std::time::Instant::now();
        let window = std::time::Duration::from_secs(self.window_seconds);

        let mut requests = self.requests.lock().await;
        let user_requests = requests.entry(subject_id.clone()).or_insert_with(Vec::new);

        // Remove old requests outside the window
        user_requests.retain(|&t| now.duration_since(t) < window);

        if user_requests.len() >= self.max_requests {
            return Err(InterceptError::internal(&format!(
                "Rate limit exceeded: max {} requests per {} seconds",
                self.max_requests, self.window_seconds
            )));
        }

        user_requests.push(now);
        Ok(StageOutcome::Continue)
    }
}

/// Policy enforcement stage bridging to soul-base auth
pub struct PolicyEnforcementStage {
    auth_facade: Arc<AuthFacade>,
}

impl PolicyEnforcementStage {
    pub fn new(auth_facade: Arc<AuthFacade>) -> Self {
        Self { auth_facade }
    }
}

#[async_trait]
impl Stage for PolicyEnforcementStage {
    async fn handle(
        &self,
        cx: &mut InterceptContext,
        _req: &mut dyn ProtoRequest,
        rsp: &mut dyn ProtoResponse,
    ) -> Result<StageOutcome, InterceptError> {
        let Some(authn_input) = cx.authn_input.clone() else {
            return write_error(
                rsp,
                InterceptError::from_public(codes::AUTH_UNAUTHENTICATED, "Please sign in."),
            )
            .await;
        };

        let Some(route) = cx.route.clone() else {
            return write_error(rsp, InterceptError::deny_policy("Route not bound")).await;
        };

        let mut attrs = route.attrs.clone();
        let tenant = cx
            .subject
            .as_ref()
            .map(|s| s.tenant.0.clone())
            .or_else(|| cx.tenant_header.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let subject_id = cx
            .subject
            .as_ref()
            .map(|s| s.subject_id.0.clone())
            .unwrap_or_else(|| "anonymous".to_string());

        merge_attrs(
            &mut attrs,
            json!({
                "browser": {
                    "tenant": tenant,
                    "session": cx.envelope_seed.partition_key.clone(),
                    "request_id": cx.request_id.clone(),
                },
                "subject": {
                    "id": subject_id,
                },
                "trace": {
                    "trace_id": cx.trace.trace_id.clone(),
                    "span_id": cx.trace.span_id.clone(),
                }
            }),
        );

        let decision = self
            .auth_facade
            .authorize(
                authn_input,
                route.resource.clone(),
                route.action.clone(),
                attrs,
                None,
                cx.envelope_seed.correlation_id.clone(),
            )
            .await
            .map_err(|e| InterceptError::from_error(e.into_inner()))?;

        if decision.allow {
            tracing::info!(
                target: "auth.audit",
                tenant = %tenant,
                subject = %subject_id,
                resource = ?route.resource,
                action = ?route.action,
                obligations = decision.obligations.len(),
                "Policy enforcement allowed request"
            );
        } else {
            tracing::warn!(
                target: "auth.audit",
                tenant = %tenant,
                subject = %subject_id,
                resource = ?route.resource,
                action = ?route.action,
                reason = ?decision.reason,
                "Policy enforcement denied request"
            );
        }

        if !decision.allow {
            let msg = decision
                .reason
                .clone()
                .unwrap_or_else(|| "Operation denied".to_string());
            return write_error(
                rsp,
                InterceptError::from_public(codes::AUTH_FORBIDDEN, &msg),
            )
            .await;
        }

        cx.obligations = decision.obligations.clone();
        Ok(StageOutcome::Continue)
    }
}

async fn write_error(
    rsp: &mut dyn ProtoResponse,
    err: InterceptError,
) -> Result<StageOutcome, InterceptError> {
    let (status, json) = soulbase_interceptors::errors::to_http_response(&err);
    rsp.set_status(status);
    rsp.write_json(&json).await?;
    Ok(StageOutcome::ShortCircuit)
}

/// Browser interceptor chain builder
pub struct BrowserInterceptorBuilder {
    stages: Vec<Box<dyn Stage>>,
}

impl BrowserInterceptorBuilder {
    pub fn new() -> Self {
        Self { stages: Vec::new() }
    }

    /// Load soul-base standard stages to align with enterprise defaults
    pub fn with_standard_stages(mut self) -> Self {
        self.stages.push(Box::new(ContextInitStage));
        self.stages.push(Box::new(TenantGuardStage));
        self.stages.push(Box::new(SchemaGuardStage));
        self.stages.push(Box::new(ErrorNormStage));
        self
    }

    /// Bind route policy specifications (must run before policy enforcement).
    pub fn with_route_policy(mut self, policy: RoutePolicy) -> Self {
        self.stages.push(Box::new(RoutePolicyStage { policy }));
        self
    }

    /// Add logging stage
    pub fn with_logging(mut self, level: LogLevel) -> Self {
        self.stages.push(Box::new(LoggingStage::new(level)));
        self
    }

    /// Configure resilience defaults used by `run_with_resilience`
    pub fn with_resilience(
        mut self,
        timeout: Duration,
        max_retries: usize,
        backoff: Duration,
    ) -> Self {
        self.stages.push(Box::new(ResilienceStage::new(
            timeout,
            max_retries,
            backoff,
        )));
        self
    }

    /// Add validation stage
    pub fn with_validation(mut self, stage: ValidationStage) -> Self {
        self.stages.push(Box::new(stage));
        self
    }

    /// Add rate limiting
    pub fn with_rate_limit(mut self, max_requests: usize, window_seconds: u64) -> Self {
        self.stages
            .push(Box::new(RateLimitStage::new(max_requests, window_seconds)));
        self
    }

    /// Enforce policy decisions through soul-base auth
    pub fn with_policy_enforcement(mut self, auth_facade: Arc<AuthFacade>) -> Self {
        self.stages
            .push(Box::new(PolicyEnforcementStage::new(auth_facade)));
        self
    }

    /// Build the interceptor chain
    pub fn build(self) -> InterceptorChain {
        InterceptorChain::new(self.stages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use soulbase_types::id::Id;
    use soulbase_types::subject::SubjectKind;

    #[tokio::test]
    async fn test_browser_request() {
        let mut request = BrowserRequest {
            method: "GET".to_string(),
            path: "/api/test".to_string(),
            headers: std::collections::HashMap::new(),
            body: Some(serde_json::json!({"test": "value"})),
        };

        assert_eq!(request.method(), "GET");
        assert_eq!(request.path(), "/api/test");

        let body = request.read_json().await.unwrap();
        assert_eq!(body["test"], "value");
    }

    #[tokio::test]
    async fn test_browser_response() {
        let mut response = BrowserResponse::new();
        response.set_status(201);
        response
            .write_json(&serde_json::json!({"result": "success"}))
            .await
            .unwrap();

        assert_eq!(response.status, 201);
        assert!(response.body.is_some());
    }

    #[test]
    fn test_interceptor_builder() {
        use soulbase_auth::AuthFacade;

        let _chain = BrowserInterceptorBuilder::new()
            .with_standard_stages()
            .with_route_policy(RoutePolicy::new(vec![]))
            .with_logging(LogLevel::Info)
            .with_policy_enforcement(Arc::new(AuthFacade::minimal()))
            .with_resilience(Duration::from_secs(5), 1, Duration::from_millis(100))
            .with_rate_limit(100, 60)
            .build();

        // Chain is built successfully
        // Actual execution would require a full context setup
    }

    #[tokio::test]
    async fn resilience_stage_applies_config_to_context() {
        let tenant = TenantId("tenant-a".to_string());
        let subject = Subject {
            kind: SubjectKind::User,
            subject_id: Id::new_random(),
            tenant: tenant.clone(),
            claims: Default::default(),
        };

        let context = BrowserInterceptContext::new(tenant, subject).inner;
        let mut request = BrowserRequest {
            method: "POST".to_string(),
            path: "/resource".to_string(),
            headers: std::collections::HashMap::new(),
            body: None,
        };
        let mut response = BrowserResponse::new();

        let chain = BrowserInterceptorBuilder::new()
            .with_resilience(Duration::from_secs(2), 3, Duration::from_millis(120))
            .build();

        chain
            .run_with_handler(context, &mut request, &mut response, move |cx, _req| {
                Box::pin(async move {
                    assert_eq!(cx.resilience.timeout, Duration::from_secs(2));
                    assert_eq!(cx.resilience.max_retries, 3);
                    assert_eq!(cx.resilience.backoff, Duration::from_millis(120));
                    Ok(json!({ "status": "ok" }))
                })
            })
            .await
            .expect("resilience config applied");

        assert_eq!(response.status, 200);
        assert!(response.body.is_some());
    }
}
