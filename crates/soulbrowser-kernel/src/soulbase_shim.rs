#![cfg(not(feature = "soulbase"))]

//! Minimal stand-ins for the soul-base crates so the kernel can be
//! built without the private dependencies. The implementations are
//! intentionally lightweight and cover only the surface used inside
//! this repository.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub mod soulbase_types {
    use super::*;

    pub mod tenant {
        use serde::{Deserialize, Serialize};

        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct TenantId(pub String);
    }

    pub mod id {
        use serde::{Deserialize, Serialize};

        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct Id(pub String);

        impl Id {
            pub fn new_random() -> Self {
                Self(uuid::Uuid::new_v4().to_string())
            }
        }

        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct CorrelationId(pub String);
    }

    pub mod time {
        use serde::{Deserialize, Serialize};

        #[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
        pub struct Timestamp(pub i64);
    }

    pub mod subject {
        use super::super::soulbase_types::id::Id;
        use super::super::soulbase_types::tenant::TenantId;
        use serde::{Deserialize, Serialize};
        use std::collections::HashMap;

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct Subject {
            pub kind: SubjectKind,
            pub subject_id: Id,
            pub tenant: TenantId,
            pub claims: HashMap<String, serde_json::Value>,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub enum SubjectKind {
            User,
            Service,
            System,
        }
    }

    pub mod trace {
        use serde::{Deserialize, Serialize};
        use std::collections::HashMap;

        #[derive(Clone, Debug, Serialize, Deserialize, Default)]
        pub struct TraceContext {
            pub trace_id: Option<String>,
            pub span_id: Option<String>,
            pub baggage: HashMap<String, String>,
        }
    }

    pub mod envelope {
        use super::super::soulbase_types::id::Id;
        use super::super::soulbase_types::time::Timestamp;
        use serde::{Deserialize, Serialize};

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct Envelope<T> {
            pub id: Id,
            pub timestamp: Timestamp,
            pub kind: String,
            pub subject: super::subject::Subject,
            pub version: String,
            pub payload: T,
        }

        impl<T> Envelope<T> {
            pub fn new(
                id: Id,
                ts: Timestamp,
                kind: String,
                subject: super::subject::Subject,
                version: &str,
                payload: T,
            ) -> Self {
                Self {
                    id,
                    timestamp: ts,
                    kind,
                    subject,
                    version: version.to_string(),
                    payload,
                }
            }
        }
    }
}

pub mod soulbase_errors {
    use super::*;

    pub mod code {
        #[derive(Clone, Copy, Debug)]
        pub struct ErrorCode(pub &'static str);

        pub mod codes {
            use super::ErrorCode;

            pub const AUTH_UNAUTHENTICATED: ErrorCode = ErrorCode("AUTH_UNAUTHENTICATED");
            pub const AUTH_FORBIDDEN: ErrorCode = ErrorCode("AUTH_FORBIDDEN");
            pub const STORAGE_NOT_FOUND: ErrorCode = ErrorCode("STORAGE_NOT_FOUND");
            pub const SCHEMA_VALIDATION: ErrorCode = ErrorCode("SCHEMA_VALIDATION");
            pub const LLM_TIMEOUT: ErrorCode = ErrorCode("LLM_TIMEOUT");
            pub const UNKNOWN_INTERNAL: ErrorCode = ErrorCode("UNKNOWN_INTERNAL");
        }
    }

    pub mod severity {
        #[derive(Clone, Copy, Debug)]
        pub enum Severity {
            Low,
            Medium,
            High,
        }
    }

    pub mod retry {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum RetryClass {
            None,
            Transient,
            Permanent,
        }
    }

    pub mod model {
        use super::code::ErrorCode;
        use super::retry::RetryClass;
        use super::severity::Severity;
        use serde::{Deserialize, Serialize};

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct ErrorObj {
            pub code: ErrorCode,
            pub message_user: String,
            pub message_dev: Option<String>,
            pub http_status: u16,
            pub retryable: RetryClass,
            pub severity: Severity,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct CauseEntry {
            pub code: String,
            pub summary: String,
            pub meta: Option<serde_json::Value>,
        }

        #[derive(Clone, Debug)]
        pub struct ErrorBuilder {
            code: ErrorCode,
            user_msg: Option<String>,
            dev_msg: Option<String>,
            http_status: u16,
            retryable: RetryClass,
            severity: Severity,
            cause: Option<CauseEntry>,
        }

        impl ErrorBuilder {
            pub fn new(code: ErrorCode) -> Self {
                Self {
                    code,
                    user_msg: None,
                    dev_msg: None,
                    http_status: 500,
                    retryable: RetryClass::None,
                    severity: Severity::Low,
                    cause: None,
                }
            }

            pub fn user_msg(mut self, msg: &str) -> Self {
                self.user_msg = Some(msg.to_string());
                self
            }

            pub fn dev_msg(mut self, msg: &str) -> Self {
                self.dev_msg = Some(msg.to_string());
                self
            }

            pub fn http_status(mut self, status: u16) -> Self {
                self.http_status = status;
                self
            }

            pub fn retryable(mut self, retry: RetryClass) -> Self {
                self.retryable = retry;
                self
            }

            pub fn severity(mut self, severity: Severity) -> Self {
                self.severity = severity;
                self
            }

            pub fn cause(mut self, cause: CauseEntry) -> Self {
                self.cause = Some(cause);
                self
            }

            pub fn build(self) -> ErrorObj {
                ErrorObj {
                    code: self.code,
                    message_user: self
                        .user_msg
                        .unwrap_or_else(|| "Internal error".to_string()),
                    message_dev: self.dev_msg,
                    http_status: self.http_status,
                    retryable: self.retryable,
                    severity: self.severity,
                }
            }
        }
    }

    pub mod prelude {
        pub use super::code::{codes, ErrorCode};
        pub use super::model::{CauseEntry, ErrorBuilder, ErrorObj};
        pub use super::retry::RetryClass;
        pub use super::severity::Severity;
    }
}

pub mod soulbase_config {
    use super::*;

    pub mod model {
        use serde::{Deserialize, Serialize};
        use std::collections::HashMap;

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct ConfigValue(pub serde_json::Value);

        #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
        pub struct NamespaceId(pub String);

        pub type ConfigMap = HashMap<String, ConfigValue>;
    }
}

pub mod soulbase_storage {
    use super::*;

    pub mod model {
        use super::super::soulbase_types::tenant::TenantId;

        pub trait Entity {
            const TABLE: &'static str;
            fn id(&self) -> &str;
            fn tenant(&self) -> &TenantId;
        }
    }
}

pub mod soulbase_auth {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("auth error: {message}")]
    pub struct AuthError {
        pub message: String,
    }

    impl AuthError {
        fn new(msg: &str) -> Self {
            Self {
                message: msg.to_string(),
            }
        }
    }

    pub mod model {
        use serde::{Deserialize, Serialize};
        use std::collections::HashMap;

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct ResourceUrn(pub String);

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub enum Action {
            Read,
            Write,
            List,
            Admin,
            Invoke,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub enum AuthnInput {
            BearerJwt(String),
            ApiKey(String),
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct Decision {
            pub allowed: bool,
            pub obligations: Vec<serde_json::Value>,
            pub context: HashMap<String, serde_json::Value>,
        }

        impl Decision {
            pub fn allow() -> Self {
                Self {
                    allowed: true,
                    obligations: Vec::new(),
                    context: HashMap::new(),
                }
            }
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct QuotaKey {
            pub tenant: crate::soulbase_types::tenant::TenantId,
            pub subject_id: crate::soulbase_types::id::Id,
            pub resource: ResourceUrn,
            pub action: Action,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct QuotaOutcome {
            pub allowed: bool,
            pub remaining: u64,
        }
    }

    #[derive(Default)]
    pub struct QuotaClient;

    impl QuotaClient {
        pub async fn check_and_consume(
            &self,
            _key: &model::QuotaKey,
            _cost: u32,
        ) -> Result<model::QuotaOutcome, AuthError> {
            Ok(model::QuotaOutcome {
                allowed: true,
                remaining: 1,
            })
        }
    }

    pub struct AuthFacade {
        pub quota: Arc<QuotaClient>,
    }

    impl AuthFacade {
        pub fn minimal() -> Self {
            Self {
                quota: Arc::new(QuotaClient::default()),
            }
        }

        pub async fn authorize(
            &self,
            _input: model::AuthnInput,
            _resource: model::ResourceUrn,
            _action: model::Action,
            _attrs: serde_json::Value,
            _ctx: Option<serde_json::Value>,
            _opts: Option<serde_json::Value>,
        ) -> Result<model::Decision, AuthError> {
            Ok(model::Decision::allow())
        }
    }
}

pub mod soulbase_tools {
    use super::*;

    pub mod manifest {
        use serde::{Deserialize, Serialize};

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct ToolId(pub String);

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct CapabilityDecl {
            pub domain: String,
            pub action: String,
            pub resource: String,
            pub attrs: serde_json::Value,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct ConsentPolicy {
            pub required: bool,
            pub max_ttl_ms: Option<u64>,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub enum SafetyClass {
            Low,
            Medium,
            High,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub enum SideEffect {
            None,
            Browser,
            Network,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub enum IdempoKind {
            None,
            Keyed,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub enum ConcurrencyKind {
            Queue,
            Parallel,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct Limits {
            pub timeout_ms: u64,
            pub max_bytes_in: usize,
            pub max_bytes_out: usize,
            pub max_files: usize,
            pub max_depth: usize,
            pub max_concurrency: usize,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct ToolManifest {
            pub id: ToolId,
            pub version: String,
            pub display_name: String,
            pub description: String,
            pub tags: Vec<String>,
            pub input_schema: serde_json::Value,
            pub output_schema: serde_json::Value,
            pub scopes: Vec<String>,
            pub capabilities: Vec<CapabilityDecl>,
            pub side_effect: SideEffect,
            pub safety_class: SafetyClass,
            pub consent: ConsentPolicy,
            pub limits: Limits,
            pub idempotency: IdempoKind,
            pub concurrency: ConcurrencyKind,
        }
    }

    pub mod registry {
        use super::manifest::{ToolId, ToolManifest};
        use super::*;

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct AvailableSpec {
            pub manifest: ToolManifest,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct ListFilter {
            pub tag: Option<String>,
            pub include_disabled: bool,
        }

        #[async_trait]
        pub trait ToolRegistry: Send + Sync {
            async fn upsert(
                &self,
                tenant: &crate::soulbase_types::tenant::TenantId,
                manifest: ToolManifest,
            ) -> Result<(), String>;

            async fn get(
                &self,
                tenant: &crate::soulbase_types::tenant::TenantId,
                id: &ToolId,
            ) -> Result<Option<AvailableSpec>, String>;

            async fn list(
                &self,
                tenant: &crate::soulbase_types::tenant::TenantId,
                filter: &ListFilter,
            ) -> Result<Vec<AvailableSpec>, String>;
        }

        #[derive(Default)]
        pub struct InMemoryRegistry {
            entries: RwLock<HashMap<String, HashMap<String, ToolManifest>>>,
        }

        impl InMemoryRegistry {
            pub fn new() -> Self {
                Self {
                    entries: RwLock::new(HashMap::new()),
                }
            }
        }

        #[async_trait]
        impl ToolRegistry for InMemoryRegistry {
            async fn upsert(
                &self,
                tenant: &crate::soulbase_types::tenant::TenantId,
                manifest: ToolManifest,
            ) -> Result<(), String> {
                let mut guard = self.entries.write().await;
                let tenant_map = guard.entry(tenant.0.clone()).or_default();
                tenant_map.insert(manifest.id.0.clone(), manifest);
                Ok(())
            }

            async fn get(
                &self,
                tenant: &crate::soulbase_types::tenant::TenantId,
                id: &ToolId,
            ) -> Result<Option<AvailableSpec>, String> {
                let guard = self.entries.read().await;
                Ok(guard
                    .get(&tenant.0)
                    .and_then(|map| map.get(&id.0).cloned())
                    .map(|manifest| AvailableSpec { manifest }))
            }

            async fn list(
                &self,
                tenant: &crate::soulbase_types::tenant::TenantId,
                filter: &ListFilter,
            ) -> Result<Vec<AvailableSpec>, String> {
                let guard = self.entries.read().await;
                let mut specs = guard
                    .get(&tenant.0)
                    .map(|map| map.values().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();

                if let Some(tag) = &filter.tag {
                    specs.retain(|manifest: &ToolManifest| manifest.tags.iter().any(|t| t == tag));
                }

                Ok(specs
                    .into_iter()
                    .map(|manifest| AvailableSpec { manifest })
                    .collect())
            }
        }
    }
}

pub mod soulbase_interceptors {
    use super::*;
    use crate::soulbase_auth::{model::Action, AuthFacade};

    pub mod errors {
        use super::*;

        #[derive(Debug, thiserror::Error)]
        #[error("intercept error: {message}")]
        pub struct InterceptError {
            pub status: u16,
            pub message: String,
        }

        impl InterceptError {
            pub fn new(status: u16, message: impl Into<String>) -> Self {
                Self {
                    status,
                    message: message.into(),
                }
            }
        }

        pub fn to_http_response(err: &InterceptError) -> (u16, serde_json::Value) {
            let payload = serde_json::json!({
                "error": {
                    "message": err.message,
                    "status": err.status,
                }
            });
            (err.status, payload)
        }
    }

    pub mod context {
        use super::*;
        use crate::soulbase_auth::model::AuthnInput;
        use crate::soulbase_types::{
            id::Id, subject::Subject, tenant::TenantId, trace::TraceContext,
        };

        #[derive(Clone, Debug, Default)]
        pub struct ResilienceConfig {
            pub timeout: Duration,
            pub max_retries: usize,
            pub backoff: Duration,
        }

        #[derive(Clone, Debug, Default)]
        pub struct EnvelopeSeed {
            pub correlation_id: Option<String>,
            pub causation_id: Option<String>,
            pub partition_key: String,
            pub produced_at_ms: i64,
        }

        #[derive(Clone, Debug, Default)]
        pub struct InterceptContext {
            pub request_id: String,
            pub trace: TraceContext,
            pub tenant_header: Option<String>,
            pub consent_token: Option<String>,
            pub route: Option<crate::soulbase_interceptors::policy::model::RoutePolicySpec>,
            pub subject: Option<Subject>,
            pub obligations: Vec<serde_json::Value>,
            pub envelope_seed: EnvelopeSeed,
            pub authn_input: Option<AuthnInput>,
            pub config_version: Option<String>,
            pub config_checksum: Option<String>,
            pub resilience: ResilienceConfig,
            pub extensions: HashMap<String, serde_json::Value>,
        }

        #[async_trait]
        pub trait ProtoRequest: Send {
            fn method(&self) -> &str;
            fn path(&self) -> &str;
            fn header(&self, name: &str) -> Option<String>;
            async fn read_json(&mut self) -> Result<serde_json::Value, errors::InterceptError>;
        }

        #[async_trait]
        pub trait ProtoResponse: Send {
            fn set_status(&mut self, code: u16);
            fn insert_header(&mut self, name: &str, value: &str);
            async fn write_json(
                &mut self,
                body: &serde_json::Value,
            ) -> Result<(), errors::InterceptError>;
        }
    }

    pub mod policy {
        use super::*;

        pub mod model {
            use serde::{Deserialize, Serialize};

            #[derive(Clone, Debug, Serialize, Deserialize)]
            pub enum MatchCond {
                Http { method: String, path_glob: String },
            }

            #[derive(Clone, Debug, Serialize, Deserialize)]
            pub struct RouteBindingSpec {
                pub resource: String,
                pub action: String,
                pub attrs_template: Option<serde_json::Value>,
                pub attrs_from_body: bool,
            }

            #[derive(Clone, Debug, Serialize, Deserialize)]
            pub struct RoutePolicySpec {
                pub when: MatchCond,
                pub bind: RouteBindingSpec,
            }
        }

        pub mod dsl {
            use super::model::{MatchCond, RouteBindingSpec, RoutePolicySpec};

            #[derive(Clone, Debug)]
            pub struct RoutePolicy {
                specs: Vec<RoutePolicySpec>,
            }

            impl RoutePolicy {
                pub fn new(specs: Vec<RoutePolicySpec>) -> Self {
                    Self { specs }
                }

                pub fn match_http(&self, method: &str, path: &str) -> Option<RouteBindingSpec> {
                    self.specs.iter().find_map(|spec| match &spec.when {
                        MatchCond::Http {
                            method: m,
                            path_glob,
                        } => {
                            if m.eq_ignore_ascii_case(method)
                                && (path_glob == path
                                    || path.starts_with(path_glob.trim_end_matches('*')))
                            {
                                Some(spec.bind.clone())
                            } else {
                                None
                            }
                        }
                    })
                }
            }
        }
    }

    pub mod stages {
        use super::*;
        use crate::soulbase_auth::model::{Action, ResourceUrn};
        use crate::soulbase_interceptors::context::{
            InterceptContext, ProtoRequest, ProtoResponse,
        };
        use crate::soulbase_interceptors::errors::InterceptError;
        use crate::soulbase_interceptors::policy::dsl::RoutePolicy;
        use crate::soulbase_interceptors::policy::model::RoutePolicySpec;

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum StageOutcome {
            Continue,
            ShortCircuit,
        }

        #[async_trait]
        pub trait Stage: Send + Sync {
            async fn handle(
                &self,
                cx: &mut InterceptContext,
                req: &mut dyn ProtoRequest,
                rsp: &mut dyn ProtoResponse,
            ) -> Result<StageOutcome, InterceptError>;
        }

        pub struct ContextInitStage;

        #[async_trait]
        impl Stage for ContextInitStage {
            async fn handle(
                &self,
                cx: &mut InterceptContext,
                _req: &mut dyn ProtoRequest,
                _rsp: &mut dyn ProtoResponse,
            ) -> Result<StageOutcome, InterceptError> {
                if cx.request_id.is_empty() {
                    cx.request_id = uuid::Uuid::new_v4().to_string();
                }
                Ok(StageOutcome::Continue)
            }
        }

        pub struct TenantGuardStage;

        #[async_trait]
        impl Stage for TenantGuardStage {
            async fn handle(
                &self,
                cx: &mut InterceptContext,
                _req: &mut dyn ProtoRequest,
                _rsp: &mut dyn ProtoResponse,
            ) -> Result<StageOutcome, InterceptError> {
                if cx.tenant_header.is_none() {
                    cx.tenant_header = cx.subject.as_ref().map(|subject| subject.tenant.0.clone());
                }
                Ok(StageOutcome::Continue)
            }
        }

        pub struct SchemaGuardStage;

        #[async_trait]
        impl Stage for SchemaGuardStage {
            async fn handle(
                &self,
                _cx: &mut InterceptContext,
                _req: &mut dyn ProtoRequest,
                _rsp: &mut dyn ProtoResponse,
            ) -> Result<StageOutcome, InterceptError> {
                Ok(StageOutcome::Continue)
            }
        }

        pub struct ErrorNormStage;

        #[async_trait]
        impl Stage for ErrorNormStage {
            async fn handle(
                &self,
                _cx: &mut InterceptContext,
                _req: &mut dyn ProtoRequest,
                _rsp: &mut dyn ProtoResponse,
            ) -> Result<StageOutcome, InterceptError> {
                Ok(StageOutcome::Continue)
            }
        }

        pub struct RoutePolicyStage {
            pub policy: RoutePolicy,
        }

        #[async_trait]
        impl Stage for RoutePolicyStage {
            async fn handle(
                &self,
                cx: &mut InterceptContext,
                req: &mut dyn ProtoRequest,
                _rsp: &mut dyn ProtoResponse,
            ) -> Result<StageOutcome, InterceptError> {
                match self.policy.match_http(req.method(), req.path()) {
                    Some(binding) => {
                        cx.route = Some(RoutePolicySpec {
                            when: crate::soulbase_interceptors::policy::model::MatchCond::Http {
                                method: req.method().to_string(),
                                path_glob: req.path().to_string(),
                            },
                            bind: binding,
                        });
                        Ok(StageOutcome::Continue)
                    }
                    None => Err(InterceptError::new(
                        403,
                        format!("route not allowed: {} {}", req.method(), req.path()),
                    )),
                }
            }
        }

        pub struct ResilienceStage {
            timeout: Duration,
            max_retries: usize,
            backoff: Duration,
        }

        impl ResilienceStage {
            pub fn new(timeout: Duration, max_retries: usize, backoff: Duration) -> Self {
                Self {
                    timeout,
                    max_retries,
                    backoff,
                }
            }
        }

        #[async_trait]
        impl Stage for ResilienceStage {
            async fn handle(
                &self,
                cx: &mut InterceptContext,
                _req: &mut dyn ProtoRequest,
                _rsp: &mut dyn ProtoResponse,
            ) -> Result<StageOutcome, InterceptError> {
                cx.resilience.timeout = self.timeout;
                cx.resilience.max_retries = self.max_retries;
                cx.resilience.backoff = self.backoff;
                Ok(StageOutcome::Continue)
            }
        }

        pub struct ValidationStage;

        #[async_trait]
        impl Stage for ValidationStage {
            async fn handle(
                &self,
                _cx: &mut InterceptContext,
                _req: &mut dyn ProtoRequest,
                _rsp: &mut dyn ProtoResponse,
            ) -> Result<StageOutcome, InterceptError> {
                Ok(StageOutcome::Continue)
            }
        }

        pub struct RateLimitStage {
            max_requests: usize,
            window: Duration,
            buckets: Mutex<HashMap<String, Vec<Instant>>>,
        }

        impl RateLimitStage {
            pub fn new(max_requests: usize, window_seconds: u64) -> Self {
                Self {
                    max_requests,
                    window: Duration::from_secs(window_seconds),
                    buckets: Mutex::new(HashMap::new()),
                }
            }
        }

        #[async_trait]
        impl Stage for RateLimitStage {
            async fn handle(
                &self,
                cx: &mut InterceptContext,
                _req: &mut dyn ProtoRequest,
                rsp: &mut dyn ProtoResponse,
            ) -> Result<StageOutcome, InterceptError> {
                let tenant = cx
                    .tenant_header
                    .clone()
                    .unwrap_or_else(|| "anonymous".to_string());
                let mut guard = self.buckets.lock().unwrap();
                let now = Instant::now();
                let entries = guard.entry(tenant.clone()).or_default();
                entries.retain(|ts| now.duration_since(*ts) < self.window);
                if entries.len() >= self.max_requests {
                    rsp.set_status(429);
                    return Err(InterceptError::new(429, "rate limit exceeded"));
                }
                entries.push(now);
                Ok(StageOutcome::Continue)
            }
        }

        pub struct PolicyEnforcementStage {
            auth: Arc<AuthFacade>,
        }

        impl PolicyEnforcementStage {
            pub fn new(auth: Arc<AuthFacade>) -> Self {
                Self { auth }
            }
        }

        #[async_trait]
        impl Stage for PolicyEnforcementStage {
            async fn handle(
                &self,
                cx: &mut InterceptContext,
                _req: &mut dyn ProtoRequest,
                _rsp: &mut dyn ProtoResponse,
            ) -> Result<StageOutcome, InterceptError> {
                let binding = cx
                    .route
                    .as_ref()
                    .ok_or_else(|| InterceptError::new(500, "missing route binding"))?;
                let subject = cx
                    .subject
                    .as_ref()
                    .ok_or_else(|| InterceptError::new(401, "missing subject"))?;
                let action = match binding.bind.action.as_str() {
                    "Read" => Action::Read,
                    "Write" => Action::Write,
                    "List" => Action::List,
                    "Admin" => Action::Admin,
                    _ => Action::Invoke,
                };
                self.auth
                    .authorize(
                        crate::soulbase_auth::model::AuthnInput::BearerJwt(
                            subject.subject_id.0.clone(),
                        ),
                        crate::soulbase_auth::model::ResourceUrn(binding.bind.resource.clone()),
                        action,
                        binding
                            .bind
                            .attrs_template
                            .clone()
                            .unwrap_or_else(|| serde_json::json!({})),
                        None,
                        None,
                    )
                    .await
                    .map_err(|err| InterceptError::new(403, err.message))?;
                Ok(StageOutcome::Continue)
            }
        }
    }

    pub struct InterceptorChain {
        stages: Vec<Box<dyn stages::Stage>>,
    }

    impl InterceptorChain {
        pub fn new(stages: Vec<Box<dyn stages::Stage>>) -> Self {
            Self { stages }
        }

        pub async fn run_with_handler<F>(
            &self,
            mut cx: context::InterceptContext,
            req: &mut dyn context::ProtoRequest,
            rsp: &mut dyn context::ProtoResponse,
            handler: F,
        ) -> Result<(), errors::InterceptError>
        where
            F: for<'a> FnOnce(
                    &'a mut context::InterceptContext,
                    &'a mut dyn context::ProtoRequest,
                ) -> std::pin::Pin<
                    Box<
                        dyn futures::Future<
                                Output = Result<serde_json::Value, errors::InterceptError>,
                            > + Send
                            + 'a,
                    >,
                > + Send,
        {
            for stage in &self.stages {
                match stage.handle(&mut cx, req, rsp).await? {
                    stages::StageOutcome::Continue => continue,
                    stages::StageOutcome::ShortCircuit => return Ok(()),
                }
            }

            let payload = handler(&mut cx, req).await?;
            rsp.write_json(&payload).await?;
            Ok(())
        }
    }
}
