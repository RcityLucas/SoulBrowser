//! Browser implementation using soul-base components
//!
//! This module provides the actual implementation of browser functionality
//! using the migrated soul-base components.

use crate::{
    auth::{AuthSession, BrowserAuthManager, SessionManager},
    config::{BrowserConfiguration, ConfigValue},
    interceptors::{BrowserInterceptorBuilder, BrowserRequest, BrowserResponse, LogLevel},
    storage::{BrowserEvent, BrowserSessionEntity, StorageManager},
    tools::BrowserToolManager,
    types::BrowserType,
};
use anyhow::{anyhow, Context, Result};
use serde_json::json;
use soulbase_interceptors::errors::InterceptError;
use soulbase_types::tenant::TenantId;
use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId};
use std::sync::Arc;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    time::{Duration, Instant},
};

/// Browser configuration
#[derive(Debug, Clone)]
pub struct BrowserConfig {
    #[allow(dead_code)]
    pub browser_type: BrowserType,
    #[allow(dead_code)]
    pub headless: bool,
    #[allow(dead_code)]
    pub window_size: Option<(u32, u32)>,
    #[allow(dead_code)]
    pub devtools: bool,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            browser_type: BrowserType::default(),
            headless: false,
            window_size: Some((1280, 720)),
            devtools: false,
        }
    }
}

/// L0 Protocol Layer - Handles soul-base configuration and setup
pub struct L0Protocol {
    #[allow(dead_code)]
    config: Arc<BrowserConfiguration>,
    tenant_id: TenantId,
}

impl L0Protocol {
    pub async fn new() -> Result<Self> {
        println!("L0Protocol: Initializing with soul-base config...");
        let tenant_id = TenantId("default-tenant".to_string());
        let mut config = BrowserConfiguration::new();

        // Set default configuration
        config.set("browser.headless".to_string(), ConfigValue::Bool(false))?;
        config.set("browser.devtools".to_string(), ConfigValue::Bool(false))?;
        config.set(
            "browser.type".to_string(),
            ConfigValue::String("chromium".to_string()),
        )?;

        Ok(Self {
            config: Arc::new(config),
            tenant_id,
        })
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    #[allow(dead_code)]
    pub fn config(&self) -> Arc<BrowserConfiguration> {
        self.config.clone()
    }
}

/// L1 Browser Manager - Manages browser lifecycle using soul-base auth and storage
pub struct L1BrowserManager {
    protocol: L0Protocol,
    auth_manager: Arc<BrowserAuthManager>,
    session_manager: Arc<SessionManager>,
    storage_manager: Arc<StorageManager>,
    tool_manager: Arc<BrowserToolManager>,
    browser_config: BrowserConfig,
}

impl L1BrowserManager {
    pub async fn new(protocol: L0Protocol, browser_config: BrowserConfig) -> Result<Self> {
        println!("L1BrowserManager: Initializing with soul-base auth and storage...");
        let tenant_id = protocol.tenant_id().0.clone();

        // Initialize soul-base components
        println!("  - Initializing soulbase-auth BrowserAuthManager...");
        let auth_manager = Arc::new(
            BrowserAuthManager::new(tenant_id.clone())
                .await
                .context("Failed to initialize auth manager")?,
        );

        println!("  - Initializing soulbase-auth SessionManager...");
        let session_manager = Arc::new(SessionManager::new());

        println!("  - Initializing soulbase-storage StorageManager...");
        let storage_manager = Arc::new(StorageManager::in_memory());

        println!("  - Initializing soulbase-tools BrowserToolManager...");
        let tool_manager = Arc::new(BrowserToolManager::new(tenant_id.clone()));

        // Register default tools
        tool_manager
            .register_default_tools()
            .await
            .context("Failed to register default tools")?;

        Ok(Self {
            protocol,
            auth_manager,
            session_manager,
            storage_manager,
            tool_manager,
            browser_config,
        })
    }

    pub async fn launch_browser(&mut self) -> Result<Browser> {
        // Authenticate and create session
        let auth_session = self
            .auth_manager
            .authenticate_token("demo-user".to_string())
            .await
            .context("Failed to authenticate")?;

        let session_id_str = self
            .session_manager
            .create_session(auth_session.clone())
            .await;
        let session_id = SessionId(session_id_str.clone());

        // Store session in soul-base storage
        let session_entity = BrowserSessionEntity {
            id: session_id_str.clone(),
            tenant: self.protocol.tenant_id().clone(),
            subject_id: auth_session.subject().subject_id.0.clone(),
            created_at: chrono::Utc::now().timestamp_millis(),
            updated_at: chrono::Utc::now().timestamp_millis(),
            state: "active".to_string(),
            metadata: serde_json::json!({}),
        };

        self.storage_manager
            .backend()
            .store_session(session_entity)
            .await
            .context("Failed to store session")?;

        Ok(Browser {
            session_id,
            tenant_id: self.protocol.tenant_id().clone(),
            auth_session,
            auth_manager: self.auth_manager.clone(),
            storage_manager: self.storage_manager.clone(),
            tool_manager: self.tool_manager.clone(),
            _config: self.browser_config.clone(),
        })
    }
}

/// Browser instance with soul-base integration
pub struct Browser {
    session_id: SessionId,
    tenant_id: TenantId,
    auth_session: AuthSession,
    auth_manager: Arc<BrowserAuthManager>,
    storage_manager: Arc<StorageManager>,
    tool_manager: Arc<BrowserToolManager>,
    _config: BrowserConfig,
}

impl Browser {
    pub async fn new_page(&self) -> Result<Page> {
        // Create a new page with interceptor chain
        let interceptor_chain = BrowserInterceptorBuilder::new()
            .with_standard_stages()
            .with_route_policy(self.auth_manager.route_policy())
            .with_logging(LogLevel::Info)
            .with_policy_enforcement(self.auth_manager.auth_facade())
            .with_resilience(Duration::from_secs(8), 2, Duration::from_millis(250))
            .with_rate_limit(100, 60)
            .build();

        let page_id = PageId::new();
        let frame_id = FrameId::new();

        Ok(Page {
            browser: self.clone_refs(),
            page_id,
            frame_id,
            url: None,
            interceptor_chain: Arc::new(interceptor_chain),
            tool_manager: self.tool_manager.clone(),
        })
    }

    fn clone_refs(&self) -> BrowserRefs {
        BrowserRefs {
            session_id: self.session_id.clone(),
            tenant_id: self.tenant_id.clone(),
            auth_session: self.auth_session.clone(),
            auth_manager: self.auth_manager.clone(),
            storage_manager: self.storage_manager.clone(),
            tool_manager: self.tool_manager.clone(),
        }
    }
}

#[derive(Clone)]
struct BrowserRefs {
    session_id: SessionId,
    tenant_id: TenantId,
    auth_session: AuthSession,
    #[allow(dead_code)]
    auth_manager: Arc<BrowserAuthManager>,
    storage_manager: Arc<StorageManager>,
    #[allow(dead_code)]
    tool_manager: Arc<BrowserToolManager>,
}

/// Page instance with soul-base interceptors
pub struct Page {
    browser: BrowserRefs,
    page_id: PageId,
    frame_id: FrameId,
    url: Option<String>,
    interceptor_chain: Arc<soulbase_interceptors::InterceptorChain>,
    tool_manager: Arc<BrowserToolManager>,
}

impl Page {
    fn session_id_str(&self) -> &str {
        &self.browser.session_id.0
    }

    fn exec_route(&self) -> ExecRoute {
        ExecRoute::new(
            self.browser.session_id.clone(),
            self.page_id.clone(),
            self.frame_id.clone(),
        )
    }

    async fn execute_with_interceptors<F>(
        &self,
        mut request: BrowserRequest,
        handler: F,
    ) -> Result<serde_json::Value>
    where
        F: for<'a> FnOnce(
                &'a mut soulbase_interceptors::context::InterceptContext,
                &'a mut dyn soulbase_interceptors::context::ProtoRequest,
            ) -> Pin<
                Box<dyn Future<Output = Result<serde_json::Value, InterceptError>> + Send + 'a>,
            > + Send,
    {
        let mut response = BrowserResponse::new();
        let context = self.create_intercept_context();
        let request_id = context.request_id.clone();

        let started_at = Instant::now();
        let method = request.method.clone();
        let path = request.path.clone();

        tracing::debug!(
            method = %method,
            path = %path,
            tenant = %self.browser.tenant_id.0,
            request_id = %request_id,
            "starting interceptor execution"
        );

        let result = self
            .interceptor_chain
            .run_with_handler(context, &mut request, &mut response, handler)
            .await;

        let elapsed_ms = started_at.elapsed().as_millis() as u64;

        match result {
            Ok(()) => {
                tracing::info!(
                    method = %method,
                    path = %path,
                    tenant = %self.browser.tenant_id.0,
                    request_id = %request_id,
                    elapsed_ms,
                    "interceptor execution completed"
                );
                Ok(response.body.unwrap_or_else(|| json!({})))
            }
            Err(err) => {
                let (status, payload) = soulbase_interceptors::errors::to_http_response(&err);
                let payload_for_log = payload.clone();
                tracing::error!(
                    method = %method,
                    path = %path,
                    tenant = %self.browser.tenant_id.0,
                    request_id = %request_id,
                    elapsed_ms,
                    error = %err,
                    http_status = status,
                    payload = %payload_for_log,
                    "interceptor execution failed"
                );
                Err(interceptor_error(err, status, payload))
            }
        }
    }

    fn create_intercept_context(&self) -> soulbase_interceptors::context::InterceptContext {
        use soulbase_interceptors::context::{EnvelopeSeed, InterceptContext};
        use soulbase_types::trace::TraceContext;

        InterceptContext {
            request_id: uuid::Uuid::new_v4().to_string(),
            trace: TraceContext {
                trace_id: Some(uuid::Uuid::new_v4().to_string()),
                span_id: Some(uuid::Uuid::new_v4().to_string()),
                baggage: Default::default(),
            },
            tenant_header: Some(self.browser.tenant_id.0.clone()),
            consent_token: None,
            route: None,
            subject: Some(self.browser.auth_session.subject().clone()),
            obligations: Vec::new(),
            envelope_seed: EnvelopeSeed {
                correlation_id: Some(uuid::Uuid::new_v4().to_string()),
                causation_id: None,
                partition_key: self.session_id_str().to_string(),
                produced_at_ms: chrono::Utc::now().timestamp_millis(),
            },
            authn_input: Some(self.browser.auth_session.authn_input()),
            config_version: None,
            config_checksum: None,
            resilience: Default::default(),
            extensions: Default::default(),
        }
    }

    pub async fn navigate(&mut self, url: &str) -> Result<()> {
        let _resource = format!("browser://session/{}/navigate", self.session_id_str());
        let policy_path = "browser://session/navigate".to_string();
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let request = BrowserRequest {
            method: "POST".to_string(),
            path: policy_path,
            headers,
            body: Some(json!({
                "session_id": self.session_id_str(),
                "url": url
            })),
        };

        let storage_manager = self.browser.storage_manager.clone();
        let tool_manager = self.tool_manager.clone();
        let tenant = self.browser.tenant_id.clone();
        let session_id = self.session_id_str().to_string();
        let subject_id = self.browser.auth_session.subject().subject_id.0.clone();
        let url_owned = url.to_string();
        let exec_route = self.exec_route();

        self.execute_with_interceptors(request, move |_cx, _req| {
            let storage_manager = storage_manager.clone();
            let tool_manager = tool_manager.clone();
            let tenant = tenant.clone();
            let session_id = session_id.clone();
            let subject_id_for_event = subject_id.clone();
            let subject_id_for_tool = subject_id.clone();
            let url_for_event = url_owned.clone();
            let url_for_tool = url_owned.clone();
            let exec_route_for_tool = exec_route.clone();

            Box::pin(async move {
                let event = BrowserEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    tenant: tenant.clone(),
                    session_id: session_id.clone(),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                    event_type: "navigate".to_string(),
                    data: json!({ "url": url_for_event, "subject": subject_id_for_event }),
                    sequence: 1,
                    tags: vec!["navigation".to_string()],
                };

                storage_manager
                    .backend()
                    .store_event(event)
                    .await
                    .map_err(|e| {
                        InterceptError::internal(&format!(
                            "failed to store navigation event: {}",
                            e
                        ))
                    })?;

                let tool_result = tool_manager
                    .execute_with_route(
                        "navigate-to-url",
                        &subject_id_for_tool,
                        json!({ "url": url_for_tool }),
                        Some(exec_route_for_tool),
                        None,
                    )
                    .await
                    .map_err(|e| {
                        InterceptError::internal(&format!("tool execution failed: {}", e))
                    })?;

                Ok(json!({ "status": "navigated", "tool": tool_result }))
            })
        })
        .await?;

        self.url = Some(url.to_string());
        Ok(())
    }

    pub async fn click(&mut self, selector: &str) -> Result<()> {
        let _resource = format!("browser://session/{}/click", self.session_id_str());
        let policy_path = "browser://session/click".to_string();
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let request = BrowserRequest {
            method: "POST".to_string(),
            path: policy_path,
            headers,
            body: Some(json!({
                "session_id": self.session_id_str(),
                "selector": selector
            })),
        };

        let storage_manager = self.browser.storage_manager.clone();
        let tool_manager = self.tool_manager.clone();
        let tenant = self.browser.tenant_id.clone();
        let session_id = self.session_id_str().to_string();
        let selector_owned = selector.to_string();
        let subject_id = self.browser.auth_session.subject().subject_id.0.clone();
        let exec_route = self.exec_route();

        self.execute_with_interceptors(request, move |_cx, _req| {
            let storage_manager = storage_manager.clone();
            let tool_manager = tool_manager.clone();
            let tenant = tenant.clone();
            let session_id = session_id.clone();
            let selector_for_event = selector_owned.clone();
            let selector_for_tool = selector_owned.clone();
            let subject_for_tool = subject_id.clone();
            let exec_route_for_tool = exec_route.clone();

            Box::pin(async move {
                let event = BrowserEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    tenant: tenant.clone(),
                    session_id: session_id.clone(),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                    event_type: "click".to_string(),
                    data: json!({ "selector": selector_for_event }),
                    sequence: 2,
                    tags: vec!["interaction".to_string()],
                };

                storage_manager
                    .backend()
                    .store_event(event)
                    .await
                    .map_err(|e| {
                        InterceptError::internal(&format!("failed to store click event: {}", e))
                    })?;

                let tool_result = tool_manager
                    .execute_with_route(
                        "click",
                        &subject_for_tool,
                        json!({
                            "anchor": {
                                "strategy": "css",
                                "selector": selector_for_tool
                            }
                        }),
                        Some(exec_route_for_tool),
                        None,
                    )
                    .await
                    .map_err(|e| {
                        InterceptError::internal(&format!("tool execution failed: {}", e))
                    })?;

                Ok(json!({ "status": "clicked", "tool": tool_result }))
            })
        })
        .await?;

        Ok(())
    }

    pub async fn type_text(&mut self, selector: &str, text: &str) -> Result<()> {
        let _resource = format!("browser://session/{}/type", self.session_id_str());
        let policy_path = "browser://session/type".to_string();
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let request = BrowserRequest {
            method: "POST".to_string(),
            path: policy_path,
            headers,
            body: Some(json!({
                "session_id": self.session_id_str(),
                "selector": selector,
                "text": text
            })),
        };

        let storage_manager = self.browser.storage_manager.clone();
        let tool_manager = self.tool_manager.clone();
        let tenant = self.browser.tenant_id.clone();
        let session_id = self.session_id_str().to_string();
        let selector_owned = selector.to_string();
        let text_owned = text.to_string();
        let subject_id = self.browser.auth_session.subject().subject_id.0.clone();
        let exec_route = self.exec_route();

        self.execute_with_interceptors(request, move |_cx, _req| {
            let storage_manager = storage_manager.clone();
            let tool_manager = tool_manager.clone();
            let tenant = tenant.clone();
            let session_id = session_id.clone();
            let selector_for_event = selector_owned.clone();
            let text_for_event = text_owned.clone();
            let selector_for_tool = selector_owned.clone();
            let text_for_tool = text_owned.clone();
            let subject_for_tool = subject_id.clone();
            let exec_route_for_tool = exec_route.clone();

            Box::pin(async move {
                let event = BrowserEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    tenant: tenant.clone(),
                    session_id: session_id.clone(),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                    event_type: "type".to_string(),
                    data: json!({
                        "selector": selector_for_event,
                        "text": text_for_event
                    }),
                    sequence: 3,
                    tags: vec!["input".to_string()],
                };

                storage_manager
                    .backend()
                    .store_event(event)
                    .await
                    .map_err(|e| {
                        InterceptError::internal(&format!("failed to store type event: {}", e))
                    })?;

                let tool_result = tool_manager
                    .execute_with_route(
                        "type-text",
                        &subject_for_tool,
                        json!({
                            "anchor": {
                                "strategy": "css",
                                "selector": selector_for_tool
                            },
                            "text": text_for_tool,
                            "wait_tier": "domready"
                        }),
                        Some(exec_route_for_tool),
                        None,
                    )
                    .await
                    .map_err(|e| {
                        InterceptError::internal(&format!("tool execution failed: {}", e))
                    })?;

                Ok(json!({ "status": "typed", "tool": tool_result }))
            })
        })
        .await?;

        Ok(())
    }

    pub async fn select_option(
        &mut self,
        selector: &str,
        value: &str,
        match_kind: Option<&str>,
        mode: Option<&str>,
    ) -> Result<()> {
        let _resource = format!("browser://session/{}/select", self.session_id_str());
        let policy_path = "browser://session/select".to_string();
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let request = BrowserRequest {
            method: "POST".to_string(),
            path: policy_path,
            headers,
            body: Some(json!({
                "session_id": self.session_id_str(),
                "selector": selector,
                "value": value,
                "match_kind": match_kind,
                "mode": mode
            })),
        };

        let storage_manager = self.browser.storage_manager.clone();
        let tool_manager = self.tool_manager.clone();
        let tenant = self.browser.tenant_id.clone();
        let session_id = self.session_id_str().to_string();
        let selector_owned = selector.to_string();
        let value_owned = value.to_string();
        let match_owned = match_kind.map(|s| s.to_string());
        let mode_owned = mode.map(|s| s.to_string());
        let subject_id = self.browser.auth_session.subject().subject_id.0.clone();
        let exec_route = self.exec_route();

        self.execute_with_interceptors(request, move |_cx, _req| {
            let storage_manager = storage_manager.clone();
            let tool_manager = tool_manager.clone();
            let tenant = tenant.clone();
            let session_id = session_id.clone();
            let selector_for_event = selector_owned.clone();
            let value_for_event = value_owned.clone();
            let match_for_event = match_owned.clone();
            let mode_for_event = mode_owned.clone();
            let selector_for_tool = selector_owned.clone();
            let value_for_tool = value_owned.clone();
            let match_for_tool = match_owned.clone();
            let mode_for_tool = mode_owned.clone();
            let subject_for_tool = subject_id.clone();
            let exec_route_for_tool = exec_route.clone();

            Box::pin(async move {
                let event = BrowserEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    tenant: tenant.clone(),
                    session_id: session_id.clone(),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                    event_type: "select".to_string(),
                    data: json!({
                        "selector": selector_for_event,
                        "value": value_for_event,
                        "match_kind": match_for_event,
                        "mode": mode_for_event
                    }),
                    sequence: 4,
                    tags: vec!["interaction".to_string()],
                };

                storage_manager
                    .backend()
                    .store_event(event)
                    .await
                    .map_err(|e| {
                        InterceptError::internal(&format!("failed to store select event: {}", e))
                    })?;

                let mut payload = serde_json::Map::new();
                payload.insert(
                    "anchor".to_string(),
                    json!({ "strategy": "css", "selector": selector_for_tool }),
                );
                payload.insert("value".to_string(), json!(value_for_tool));
                if let Some(kind) = match_for_tool {
                    payload.insert("match_kind".to_string(), json!(kind));
                }
                if let Some(mode_value) = mode_for_tool {
                    payload.insert("mode".to_string(), json!(mode_value));
                }

                let tool_result = tool_manager
                    .execute_with_route(
                        "select-option",
                        &subject_for_tool,
                        serde_json::Value::Object(payload),
                        Some(exec_route_for_tool),
                        None,
                    )
                    .await
                    .map_err(|e| {
                        InterceptError::internal(&format!("tool execution failed: {}", e))
                    })?;

                Ok(json!({ "status": "selected", "tool": tool_result }))
            })
        })
        .await?;

        Ok(())
    }

    pub async fn screenshot(&mut self, filename: &str) -> Result<Vec<u8>> {
        let _resource = format!("browser://session/{}/screenshot", self.session_id_str());
        let policy_path = "browser://session/screenshot".to_string();
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let request = BrowserRequest {
            method: "POST".to_string(),
            path: policy_path,
            headers,
            body: Some(json!({
                "session_id": self.session_id_str(),
                "filename": filename
            })),
        };

        let storage_manager = self.browser.storage_manager.clone();
        let tool_manager = self.tool_manager.clone();
        let tenant = self.browser.tenant_id.clone();
        let session_id = self.session_id_str().to_string();
        let filename_owned = filename.to_string();
        let subject_id = self.browser.auth_session.subject().subject_id.0.clone();
        let exec_route = self.exec_route();

        let payload = self
            .execute_with_interceptors(request, move |_cx, _req| {
                let storage_manager = storage_manager.clone();
                let tool_manager = tool_manager.clone();
                let tenant = tenant.clone();
                let session_id = session_id.clone();
                let filename_for_event = filename_owned.clone();
                let filename_for_tool = filename_owned.clone();
                let subject_for_tool = subject_id.clone();
                let exec_route_for_tool = exec_route.clone();

                Box::pin(async move {
                    let event = BrowserEvent {
                        id: uuid::Uuid::new_v4().to_string(),
                        tenant: tenant.clone(),
                        session_id: session_id.clone(),
                        timestamp: chrono::Utc::now().timestamp_millis(),
                        event_type: "screenshot".to_string(),
                        data: json!({ "filename": filename_for_event }),
                        sequence: 5,
                        tags: vec!["capture".to_string()],
                    };

                    storage_manager
                        .backend()
                        .store_event(event)
                        .await
                        .map_err(|e| {
                            InterceptError::internal(&format!(
                                "failed to store screenshot event: {}",
                                e
                            ))
                        })?;

                    let tool_result = tool_manager
                        .execute_with_route(
                            "take-screenshot",
                            &subject_for_tool,
                            json!({ "filename": filename_for_tool }),
                            Some(exec_route_for_tool),
                            None,
                        )
                        .await
                        .map_err(|e| {
                            InterceptError::internal(&format!("tool execution failed: {}", e))
                        })?;

                    let byte_len = tool_result
                        .get("output")
                        .and_then(|output| output.get("byte_len"))
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);

                    Ok(json!({
                        "status": "captured",
                        "byte_len": byte_len,
                        "tool": tool_result
                    }))
                })
            })
            .await?;

        let tool_result = payload.get("tool");
        let bytes = tool_result
            .and_then(|tool| tool.get("output"))
            .and_then(|output| output.get("bytes"))
            .and_then(|value| value.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u8))
                    .collect::<Vec<u8>>()
            })
            .unwrap_or_default();

        if bytes.is_empty() {
            tracing::warn!(
                session = %self.browser.session_id.0,
                page = %self.page_id.0,
                "CDP screenshot bytes unavailable; returning placeholder buffer",
            );
            Ok(vec![0u8; 1024])
        } else {
            Ok(bytes)
        }
    }
}

fn interceptor_error(
    err: InterceptError,
    status: u16,
    payload: serde_json::Value,
) -> anyhow::Error {
    let inner = err.into_inner();
    let mut error = anyhow!("Interceptor execution failed: {:?}", inner);
    error = error.context(format!("http_status={status}"));
    error.context(format!("payload={payload}"))
}
