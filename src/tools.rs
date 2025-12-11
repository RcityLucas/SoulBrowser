//! Browser automation tools module
#![allow(dead_code)]
//!
//! Provides browser automation tools using soulbase-tools

use crate::errors::SoulBrowserError;
use crate::parsers::{
    parse_facebook_feed, parse_github_repos, parse_hackernews_feed, parse_linkedin_profile,
    parse_market_info, parse_news_brief, parse_twitter_feed,
};
use crate::structured_output::validate_structured_output;
use action_primitives::{
    ActionPrimitives, AnchorDescriptor, DefaultActionPrimitives, DefaultWaitStrategy, ExecCtx,
    ScrollBehavior, ScrollTarget, SelectMethod, WaitCondition, WaitTier,
};
use cdp_adapter::{event_bus, Cdp, CdpAdapter, CdpConfig};
use dashmap::DashMap;
use l6_observe::{guard::LabelMap as ObsLabelMap, metrics as obs_metrics, tracing as obs_tracing};
use once_cell::sync::Lazy;
use schemars::schema::{RootSchema, SchemaObject};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use soulbase_tools::{
    manifest::{
        CapabilityDecl, ConcurrencyKind, ConsentPolicy, IdempoKind, Limits, SafetyClass,
        SideEffect, ToolId, ToolManifest,
    },
    registry::{AvailableSpec, ListFilter},
    InMemoryRegistry, ToolRegistry,
};
use soulbase_types::tenant::TenantId;
use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId};
use soulbrowser_policy_center::{default_snapshot, PolicyView};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const METRIC_TOOL_INVOCATIONS: &str = "soul.l5.tool.invocations";
const METRIC_TOOL_LATENCY: &str = "soul.l5.tool.latency_ms";
const DEFAULT_TOOL_TIMEOUT_MS: u64 = 30_000;
const PAGE_OBSERVE_SCRIPT: &str = include_str!("scripts/page_observe.js");
const DATA_PARSE_TOOLS: &[(&str, &str)] = &[
    ("data.parse.generic", "Parse generic observation snapshot"),
    ("data.parse.market_info", "Parse market index snapshot"),
    ("data.parse.news_brief", "Parse news brief"),
    ("data.parse.twitter-feed", "Parse Twitter/X feed"),
    ("data.parse.facebook-feed", "Parse Facebook feed"),
    ("data.parse.hackernews-feed", "Parse Hacker News feed"),
    ("data.parse.linkedin-profile", "Parse LinkedIn profile"),
    ("data.parse.github-repo", "Parse GitHub repositories"),
];

static OBSERVATION_CACHE: Lazy<DashMap<String, ObservationEntry>> = Lazy::new(|| DashMap::new());

#[derive(Clone, Debug)]
struct ObservationEntry {
    data: Value,
    parsed: HashMap<String, Value>,
}

/// Browser tool manager using soulbase-tools
pub struct BrowserToolManager {
    registry: Arc<InMemoryRegistry>,
    tenant: TenantId,
    executor: Arc<dyn ToolExecutor>,
}

impl BrowserToolManager {
    /// Create new tool manager
    pub fn new(tenant_id: String) -> Self {
        Self {
            registry: Arc::new(InMemoryRegistry::new()),
            tenant: TenantId(tenant_id),
            executor: Arc::new(BrowserToolExecutor::new()),
        }
    }

    /// Register browser navigation tool
    pub async fn register_navigation_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_navigation_tool("navigate-to-url");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!("Failed to register navigation tool: {}", e))
            })?;

        Ok(())
    }

    /// Register click tool
    pub async fn register_click_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_click_tool("click");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!("Failed to register click tool: {}", e))
            })?;

        Ok(())
    }

    /// Register type text tool
    pub async fn register_type_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_type_tool("type-text");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!("Failed to register type tool: {}", e))
            })?;

        Ok(())
    }

    /// Register select option tool
    pub async fn register_select_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_select_tool("select-option");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!("Failed to register select tool: {}", e))
            })?;

        Ok(())
    }

    /// Register scroll tool
    pub async fn register_scroll_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_scroll_tool("scroll-page");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!("Failed to register scroll tool: {}", e))
            })?;

        Ok(())
    }

    /// Register wait-for-element tool
    pub async fn register_wait_for_element_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_wait_for_element_tool("wait-for-element");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!(
                    "Failed to register wait-for-element tool: {}",
                    e
                ))
            })?;

        Ok(())
    }

    /// Register wait-for-condition tool
    pub async fn register_wait_for_condition_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_wait_for_condition_tool("wait-for-condition");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!(
                    "Failed to register wait-for-condition tool: {}",
                    e
                ))
            })?;

        Ok(())
    }

    /// Register get-element-info tool
    pub async fn register_get_element_info_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_get_element_info_tool("get-element-info");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!(
                    "Failed to register get-element-info tool: {}",
                    e
                ))
            })?;

        Ok(())
    }

    /// Register retrieve-history tool
    pub async fn register_retrieve_history_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_retrieve_history_tool("retrieve-history");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!(
                    "Failed to register retrieve-history tool: {}",
                    e
                ))
            })?;

        Ok(())
    }

    /// Register complete-task tool
    pub async fn register_complete_task_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_complete_task_tool("complete-task");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!("Failed to register complete-task tool: {}", e))
            })?;

        Ok(())
    }

    /// Register report-insight tool
    pub async fn register_report_insight_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_report_insight_tool("report-insight");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!(
                    "Failed to register report-insight tool: {}",
                    e
                ))
            })?;

        Ok(())
    }

    /// Register observation tool
    pub async fn register_observation_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_observation_tool("data.extract-site");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!("Failed to register observation tool: {}", e))
            })?;

        Ok(())
    }

    /// Register structured parse tools
    pub async fn register_data_parse_tools(&self) -> Result<(), SoulBrowserError> {
        for (id, _) in DATA_PARSE_TOOLS {
            let manifest = create_parse_tool(id);
            self.registry
                .upsert(&self.tenant, manifest)
                .await
                .map_err(|e| {
                    SoulBrowserError::internal(&format!(
                        "Failed to register parse tool {}: {}",
                        id, e
                    ))
                })?;
        }
        Ok(())
    }

    /// Register structured delivery tool
    pub async fn register_deliver_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_deliver_tool("data.deliver.structured");
        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!("Failed to register deliver tool: {}", e))
            })?;
        Ok(())
    }

    /// Register legacy tool aliases for backward compatibility
    pub async fn register_legacy_aliases(&self) -> Result<(), SoulBrowserError> {
        let legacy_ids = [
            (
                "browser.navigate",
                create_navigation_tool("browser.navigate"),
            ),
            ("browser.click", create_click_tool("browser.click")),
            ("browser.type", create_type_tool("browser.type")),
            ("browser.select", create_select_tool("browser.select")),
            (
                "browser.screenshot",
                create_take_screenshot_tool("browser.screenshot"),
            ),
        ];

        for (_, manifest) in legacy_ids {
            self.registry
                .upsert(&self.tenant, manifest)
                .await
                .map_err(|e| {
                    SoulBrowserError::internal(&format!(
                        "Failed to register legacy tool alias: {}",
                        e
                    ))
                })?;
        }

        Ok(())
    }

    /// Register screenshot tool
    pub async fn register_take_screenshot_tool(&self) -> Result<(), SoulBrowserError> {
        let manifest = create_take_screenshot_tool("take-screenshot");

        self.registry
            .upsert(&self.tenant, manifest)
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!("Failed to register screenshot tool: {}", e))
            })?;

        Ok(())
    }

    /// Register all default browser tools
    pub async fn register_default_tools(&self) -> Result<(), SoulBrowserError> {
        self.register_navigation_tool().await?;
        self.register_click_tool().await?;
        self.register_type_tool().await?;
        self.register_select_tool().await?;
        self.register_scroll_tool().await?;
        self.register_wait_for_element_tool().await?;
        self.register_wait_for_condition_tool().await?;
        self.register_get_element_info_tool().await?;
        self.register_retrieve_history_tool().await?;
        self.register_take_screenshot_tool().await?;
        self.register_complete_task_tool().await?;
        self.register_report_insight_tool().await?;
        self.register_observation_tool().await?;
        self.register_data_parse_tools().await?;
        self.register_deliver_tool().await?;
        self.register_legacy_aliases().await?;

        Ok(())
    }

    /// List available tools
    pub async fn list_tools(
        &self,
        filter: Option<String>,
    ) -> Result<Vec<AvailableSpec>, SoulBrowserError> {
        let list_filter = ListFilter {
            tag: filter,
            include_disabled: false,
        };

        self.registry
            .list(&self.tenant, &list_filter)
            .await
            .map_err(|e| SoulBrowserError::internal(&format!("Failed to list tools: {}", e)))
    }

    /// Get tool by ID
    pub async fn get_tool(&self, tool_id: &str) -> Result<Option<AvailableSpec>, SoulBrowserError> {
        let id = ToolId(tool_id.to_string());

        self.registry
            .get(&self.tenant, &id)
            .await
            .map_err(|e| SoulBrowserError::internal(&format!("Failed to get tool: {}", e)))
    }

    /// Execute a tool by ID
    pub async fn execute(
        &self,
        tool_id: &str,
        subject_id: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, SoulBrowserError> {
        self.execute_with_route(tool_id, subject_id, input, None, None)
            .await
    }

    /// Execute a tool by ID with an explicit route
    pub async fn execute_with_route(
        &self,
        tool_id: &str,
        subject_id: &str,
        input: serde_json::Value,
        route: Option<ExecRoute>,
        timeout_ms: Option<u64>,
    ) -> Result<serde_json::Value, SoulBrowserError> {
        // Check if tool exists
        let id = ToolId(tool_id.to_string());
        let tool = self
            .registry
            .get(&self.tenant, &id)
            .await
            .map_err(|e| SoulBrowserError::internal(&format!("Tool not found: {}", e)))?;

        if tool.is_none() {
            return Err(SoulBrowserError::not_found(&format!(
                "Tool not found: {}",
                tool_id
            )));
        }

        // Create execution context
        let effective_timeout = timeout_ms.unwrap_or(DEFAULT_TOOL_TIMEOUT_MS);
        let context = ToolExecutionContext {
            tool_id: tool_id.to_string(),
            tenant_id: self.tenant.0.clone(),
            subject_id: subject_id.to_string(),
            input,
            timeout_ms: effective_timeout,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route,
        };

        let result = self.executor.execute(context).await?;
        Ok(serde_json::to_value(result)?)
    }
}

/// Create navigation tool manifest
fn create_navigation_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Navigate to URL".to_string(),
        description: "Navigate browser to specified URL".to_string(),
        tags: vec!["browser".to_string(), "navigation".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:navigate".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "navigate".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Browser,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 30000,
            max_bytes_in: 1024 * 1024,
            max_bytes_out: 10 * 1024 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 1,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create click tool manifest
fn create_click_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Click Element".to_string(),
        description: "Click on a page element using selector".to_string(),
        tags: vec!["browser".to_string(), "interaction".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:interact".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "click".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Browser,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 10000,
            max_bytes_in: 1024,
            max_bytes_out: 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 1,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create type text tool manifest
fn create_type_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Type Text".to_string(),
        description: "Type text into an input element".to_string(),
        tags: vec!["browser".to_string(), "input".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:interact".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "type".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Browser,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 10000,
            max_bytes_in: 10 * 1024,
            max_bytes_out: 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 1,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create select option tool manifest
fn create_select_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Select Option".to_string(),
        description: "Choose an option from a select control".to_string(),
        tags: vec!["browser".to_string(), "interaction".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:interact".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "select".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Browser,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 10_000,
            max_bytes_in: 2048,
            max_bytes_out: 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 1,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create scroll page/tool manifest
fn create_scroll_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Scroll Page".to_string(),
        description: "Scroll the page or a container to reveal targets".to_string(),
        tags: vec!["browser".to_string(), "navigation".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:interact".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "scroll".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Browser,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 5000,
            max_bytes_in: 8 * 1024,
            max_bytes_out: 2 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 1,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create wait-for-element tool manifest
fn create_wait_for_element_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Wait For Element".to_string(),
        description: "Wait until a structural element condition is met".to_string(),
        tags: vec!["browser".to_string(), "synchronization".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:wait".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "wait-element".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 60_000,
            max_bytes_in: 16 * 1024,
            max_bytes_out: 4 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 4,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create wait-for-condition tool manifest
fn create_wait_for_condition_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Wait For Condition".to_string(),
        description: "Wait until network/runtime conditions meet expectations".to_string(),
        tags: vec!["browser".to_string(), "synchronization".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:wait".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "wait-condition".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 60_000,
            max_bytes_in: 16 * 1024,
            max_bytes_out: 4 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 4,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create get-element-info tool manifest
fn create_get_element_info_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Get Element Info".to_string(),
        description: "Collect structural details about an element".to_string(),
        tags: vec!["browser".to_string(), "memory".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:inspect".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "inspect".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 5000,
            max_bytes_in: 32 * 1024,
            max_bytes_out: 32 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 2,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create retrieve-history tool manifest
fn create_retrieve_history_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Retrieve History".to_string(),
        description: "Retrieve recent tool execution or perception history".to_string(),
        tags: vec!["browser".to_string(), "memory".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:history".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "history".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 3000,
            max_bytes_in: 8 * 1024,
            max_bytes_out: 64 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 2,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create complete-task tool manifest
fn create_complete_task_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Complete Task".to_string(),
        description: "Record task completion metadata and evidence".to_string(),
        tags: vec!["metacognition".to_string(), "task".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["task:complete".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "task".to_string(),
            action: "complete".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 2000,
            max_bytes_in: 32 * 1024,
            max_bytes_out: 16 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 1,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create report-insight tool manifest
fn create_report_insight_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Report Insight".to_string(),
        description: "Record insights or observations for downstream consumers".to_string(),
        tags: vec!["metacognition".to_string(), "insight".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["task:insight".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "task".to_string(),
            action: "insight".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 2000,
            max_bytes_in: 24 * 1024,
            max_bytes_out: 16 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 4,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create screenshot tool manifest
fn create_take_screenshot_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Take Screenshot".to_string(),
        description: "Capture screenshot of current page".to_string(),
        tags: vec!["browser".to_string(), "capture".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:read".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "screenshot".to_string(),
            resource: "*".to_string(),
            attrs: serde_json::json!({}),
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 5000,
            max_bytes_in: 1024,
            max_bytes_out: 50 * 1024 * 1024, // 50MB for screenshot
            max_files: 1,
            max_depth: 0,
            max_concurrency: 1,
        },
        idempotency: IdempoKind::None,
        concurrency: ConcurrencyKind::Parallel,
    }
}

fn create_observation_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Capture Page Observation".to_string(),
        description: "Capture DOM/text snapshot for downstream parsing".to_string(),
        tags: vec!["data".to_string(), "observe".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["browser:read".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "browser".to_string(),
            action: "observe".to_string(),
            resource: "*".to_string(),
            attrs: Value::Null,
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 30_000,
            max_bytes_in: 4 * 1024,
            max_bytes_out: 2 * 1024 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 1,
        },
        idempotency: IdempoKind::Keyed,
        concurrency: ConcurrencyKind::Queue,
    }
}

fn create_parse_tool(id: &str) -> ToolManifest {
    let description = DATA_PARSE_TOOLS
        .iter()
        .find(|(tool_id, _)| *tool_id == id)
        .map(|(_, desc)| *desc)
        .unwrap_or("Parse structured observation data");

    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: format!("{}", id.replace("data.parse.", "Parse ")),
        description: description.to_string(),
        tags: vec!["data".to_string(), "parse".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["data:read".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "data".to_string(),
            action: "parse".to_string(),
            resource: id.to_string(),
            attrs: Value::Null,
        }],
        side_effect: SideEffect::Read,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 15_000,
            max_bytes_in: 2 * 1024 * 1024,
            max_bytes_out: 512 * 1024,
            max_files: 0,
            max_depth: 0,
            max_concurrency: 2,
        },
        idempotency: IdempoKind::Keyed,
        concurrency: ConcurrencyKind::Parallel,
    }
}

fn create_deliver_tool(id: &str) -> ToolManifest {
    ToolManifest {
        id: ToolId(id.to_string()),
        version: "1.0.0".to_string(),
        display_name: "Deliver Structured Artifact".to_string(),
        description: "Persist structured data to the artifacts directory".to_string(),
        tags: vec!["data".to_string(), "deliver".to_string()],
        input_schema: create_simple_schema(),
        output_schema: create_simple_schema(),
        scopes: vec!["data:write".to_string()],
        capabilities: vec![CapabilityDecl {
            domain: "data".to_string(),
            action: "deliver".to_string(),
            resource: "structured".to_string(),
            attrs: Value::Null,
        }],
        side_effect: SideEffect::Write,
        safety_class: SafetyClass::Low,
        consent: ConsentPolicy {
            required: false,
            max_ttl_ms: None,
        },
        limits: Limits {
            timeout_ms: 20_000,
            max_bytes_in: 2 * 1024 * 1024,
            max_bytes_out: 64 * 1024,
            max_files: 2,
            max_depth: 0,
            max_concurrency: 1,
        },
        idempotency: IdempoKind::Keyed,
        concurrency: ConcurrencyKind::Queue,
    }
}

/// Create a simple schema for tool manifests
fn create_simple_schema() -> RootSchema {
    let mut schema = RootSchema::default();
    schema.schema = SchemaObject::default().into();
    schema
}

/// Tool execution context
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolExecutionContext {
    pub tool_id: String,
    pub tenant_id: String,
    pub subject_id: String,
    pub input: serde_json::Value,
    pub timeout_ms: u64,
    pub trace_id: String,
    pub route: Option<ExecRoute>,
}

/// Tool execution result
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolExecutionResult {
    pub success: bool,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

/// Tool executor trait
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a tool
    async fn execute(
        &self,
        context: ToolExecutionContext,
    ) -> Result<ToolExecutionResult, SoulBrowserError>;

    /// Access the underlying CDP adapter when available
    fn cdp_adapter(&self) -> Option<Arc<CdpAdapter>> {
        None
    }
}

/// Browser tool executor implementation
pub struct BrowserToolExecutor {
    adapter: Arc<CdpAdapter>,
    primitives: Arc<DefaultActionPrimitives>,
    policy_view: Arc<PolicyView>,
    adapter_ready: OnceCell<()>,
    route_pages: DashMap<String, cdp_adapter::PageId>,
    page_routes: DashMap<cdp_adapter::PageId, String>,
    pending_pages: DashMap<String, Instant>,
}

impl BrowserToolExecutor {
    pub fn new() -> Self {
        obs_tracing::init_tracing();
        let (event_bus, _rx) = event_bus(64);
        let adapter = Arc::new(CdpAdapter::new(CdpConfig::default(), event_bus));
        let wait_strategy = Arc::new(DefaultWaitStrategy::default());
        let primitives = Arc::new(DefaultActionPrimitives::new(adapter.clone(), wait_strategy));
        let policy_view = Arc::new(PolicyView::from(default_snapshot()));

        Self {
            adapter,
            primitives,
            policy_view,
            adapter_ready: OnceCell::new(),
            route_pages: DashMap::new(),
            page_routes: DashMap::new(),
            pending_pages: DashMap::new(),
        }
    }

    async fn ensure_adapter_started(&self) -> Result<(), SoulBrowserError> {
        self.adapter_ready
            .get_or_try_init(|| async {
                Arc::clone(&self.adapter).start().await.map_err(|err| {
                    SoulBrowserError::internal(&format!("Failed to start CDP adapter: {}", err))
                })
            })
            .await
            .map(|_| ())
    }

    fn finish_tool(
        &self,
        context: &ToolExecutionContext,
        start: Instant,
        span: &tracing::Span,
        result: ToolExecutionResult,
    ) -> ToolExecutionResult {
        let duration_ms = start.elapsed().as_millis() as u64;
        obs_tracing::observe_latency(span, duration_ms);

        let mut labels: ObsLabelMap = ObsLabelMap::new();
        labels.insert("tool".into(), context.tool_id.clone());
        labels.insert("success".into(), result.success.to_string());
        obs_metrics::inc(METRIC_TOOL_INVOCATIONS, labels.clone());
        obs_metrics::observe(METRIC_TOOL_LATENCY, duration_ms, labels);

        result
    }

    fn record_error(&self, context: &ToolExecutionContext, start: Instant, span: &tracing::Span) {
        let duration_ms = start.elapsed().as_millis() as u64;
        obs_tracing::observe_latency(span, duration_ms);

        let mut labels: ObsLabelMap = ObsLabelMap::new();
        labels.insert("tool".into(), context.tool_id.clone());
        labels.insert("success".into(), "false".into());
        obs_metrics::inc(METRIC_TOOL_INVOCATIONS, labels.clone());
        obs_metrics::observe(METRIC_TOOL_LATENCY, duration_ms, labels);
    }

    async fn resolve_page_for_route(
        &self,
        route: &ExecRoute,
    ) -> Result<cdp_adapter::PageId, SoulBrowserError> {
        let route_key = route.page.0.clone();

        if let Some(existing) = self.route_pages.get(&route_key) {
            return Ok(*existing.value());
        }

        self.ensure_adapter_started().await?;

        let deadline = Instant::now() + Duration::from_secs(5);

        loop {
            if Instant::now() >= deadline {
                return Err(SoulBrowserError::internal(
                    "No available CDP pages for execution route",
                ));
            }

            self.cleanup_stale_mappings();

            if let Some(existing) = self.route_pages.get(&route_key) {
                return Ok(*existing.value());
            }

            let claimed: HashSet<cdp_adapter::PageId> =
                self.page_routes.iter().map(|entry| *entry.key()).collect();

            let candidate = self
                .adapter
                .registry()
                .iter()
                .into_iter()
                .filter(|(_, ctx)| ctx.cdp_session.is_some())
                .map(|(page, _)| page)
                .find(|page| !claimed.contains(page));

            if let Some(page) = candidate {
                self.route_pages.insert(route_key.clone(), page);
                self.page_routes.insert(page, route_key.clone());
                return Ok(page);
            } else {
                if self.pending_pages.get(&route_key).is_some() {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }

                self.pending_pages.insert(route_key.clone(), Instant::now());

                let create_result = self.adapter.create_page("about:blank").await;
                self.pending_pages.remove(&route_key);

                match create_result {
                    Ok(_) => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                    Err(err) => {
                        if self.adapter.registry().iter().is_empty() {
                            let synthetic = Self::synthetic_page_id(route);
                            self.route_pages.insert(route_key.clone(), synthetic);
                            return Ok(synthetic);
                        }

                        return Err(SoulBrowserError::internal(&format!(
                            "Failed to create CDP page: {}",
                            err
                        )));
                    }
                }
            }
        }
    }

    fn cleanup_stale_mappings(&self) {
        let active_pages: HashSet<cdp_adapter::PageId> = self
            .adapter
            .registry()
            .iter()
            .into_iter()
            .map(|(page, _)| page)
            .collect();

        if active_pages.is_empty() {
            return;
        }

        self.route_pages
            .retain(|_, page| active_pages.contains(page));
        self.page_routes
            .retain(|page, _| active_pages.contains(page));
    }

    fn synthetic_page_id(route: &ExecRoute) -> cdp_adapter::PageId {
        match Uuid::parse_str(&route.page.0) {
            Ok(id) => cdp_adapter::PageId(id),
            Err(_) => cdp_adapter::PageId(Uuid::new_v4()),
        }
    }

    fn build_exec_ctx(&self, route: Option<&ExecRoute>, timeout_ms: u64) -> ExecCtx {
        let route = route.cloned().unwrap_or_else(|| {
            let session = SessionId::new();
            let page = PageId::new();
            let frame = FrameId::new();
            ExecRoute::new(session, page, frame)
        });
        let timeout = if timeout_ms == 0 { 30_000 } else { timeout_ms };
        let deadline = Instant::now() + Duration::from_millis(timeout);

        ExecCtx::new(
            route,
            deadline,
            CancellationToken::new(),
            (*self.policy_view).clone(),
        )
    }

    fn parse_wait_tier(value: Option<&serde_json::Value>) -> WaitTier {
        match value
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase())
        {
            Some(ref tier) if tier == "none" => WaitTier::None,
            Some(ref tier) if tier == "idle" => WaitTier::Idle,
            Some(ref tier) if tier == "domready" => WaitTier::DomReady,
            _ => WaitTier::DomReady,
        }
    }

    fn parse_select_method(value: Option<&str>) -> Result<SelectMethod, SoulBrowserError> {
        match value.map(|v| v.to_ascii_lowercase()) {
            None => Ok(SelectMethod::default()),
            Some(kind) if kind == "value" => Ok(SelectMethod::Value),
            Some(kind) if kind == "text" || kind == "label" => Ok(SelectMethod::Text),
            Some(kind) if kind == "index" => Ok(SelectMethod::Index),
            Some(other) => Err(SoulBrowserError::validation_error(
                "Invalid match kind",
                &format!(
                    "Unsupported select match kind '{}'. Expected value, text, or index.",
                    other
                ),
            )),
        }
    }

    async fn execute_observation_tool(
        &self,
        context: &ToolExecutionContext,
        start: Instant,
        span: &tracing::Span,
        exec_ctx: ExecCtx,
    ) -> Result<ToolExecutionResult, SoulBrowserError> {
        self.ensure_adapter_started().await?;
        let page_id = self.resolve_page_for_route(&exec_ctx.route).await?;
        let raw = self
            .adapter
            .evaluate_script(page_id, PAGE_OBSERVE_SCRIPT)
            .await
            .map_err(|err| {
                self.record_error(context, start, span);
                SoulBrowserError::internal(&format!("Observation script failed: {}", err))
            })?;

        let ok = raw.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            let reason = raw
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            self.record_error(context, start, span);
            return Err(SoulBrowserError::internal(&format!(
                "Observation script reported failure: {}",
                reason
            )));
        }

        let data_value = raw.get("data").cloned().unwrap_or(Value::Null);
        OBSERVATION_CACHE.insert(
            context.subject_id.clone(),
            ObservationEntry {
                data: data_value.clone(),
                parsed: HashMap::new(),
            },
        );

        let mut metadata = serde_json::Map::new();
        metadata.insert("subject_id".to_string(), json!(context.subject_id.clone()));
        let output = ToolExecutionResult {
            success: true,
            output: Some(json!({
                "status": "captured",
                "observation": data_value,
                "preview": raw.get("preview").cloned(),
            })),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
            metadata,
        };
        Ok(self.finish_tool(context, start, span, output))
    }

    async fn execute_parse_tool(
        &self,
        context: &ToolExecutionContext,
        start: Instant,
        span: &tracing::Span,
        tool_id: &str,
    ) -> Result<ToolExecutionResult, SoulBrowserError> {
        let mut entry = OBSERVATION_CACHE
            .get_mut(&context.subject_id)
            .ok_or_else(|| {
                SoulBrowserError::internal(
                    "No observation available for this route; run data.extract-site first",
                )
            })?;

        let (schema, parsed) = self
            .run_data_parser(tool_id, &entry.data, &context.input)
            .map_err(|err| {
                self.record_error(context, start, span);
                err
            })?;
        entry.parsed.insert(schema.clone(), parsed.clone());
        drop(entry);

        let mut metadata = serde_json::Map::new();
        metadata.insert("schema".into(), json!(schema.clone()));
        let result = ToolExecutionResult {
            success: true,
            output: Some(json!({
                "status": "parsed",
                "schema": schema,
                "result": parsed,
            })),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
            metadata,
        };
        Ok(self.finish_tool(context, start, span, result))
    }

    async fn execute_deliver_tool(
        &self,
        context: &ToolExecutionContext,
        start: Instant,
        span: &tracing::Span,
    ) -> Result<ToolExecutionResult, SoulBrowserError> {
        let schema = context
            .input
            .get("schema")
            .and_then(|v| v.as_str())
            .unwrap_or("generic_observation_v1");
        let mut entry = OBSERVATION_CACHE
            .get_mut(&context.subject_id)
            .ok_or_else(|| {
                SoulBrowserError::internal(
                    "No parsed observation available; parse before delivering",
                )
            })?;
        let parsed_value = entry
            .parsed
            .get(schema)
            .or_else(|| entry.parsed.values().next())
            .cloned()
            .ok_or_else(|| {
                SoulBrowserError::internal(
                    "Missing parsed data for requested schema; ensure data.parse.* ran",
                )
            })?;
        drop(entry);

        validate_structured_output(schema, &parsed_value).map_err(|err| {
            self.record_error(context, start, span);
            SoulBrowserError::validation_error("Invalid structured output", &err.to_string())
        })?;

        let task_id = context
            .input
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("anonymous");
        let artifact_label = context
            .input
            .get("artifact_label")
            .and_then(|v| v.as_str())
            .unwrap_or("structured");
        let filename = context
            .input
            .get("filename")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("{}_{}.json", artifact_label, schema)));

        let root = PathBuf::from("soulbrowser-output")
            .join("artifacts")
            .join(task_id);
        fs::create_dir_all(&root).await.map_err(|err| {
            SoulBrowserError::internal(&format!("Failed to prepare artifact dir: {}", err))
        })?;
        let artifact_path = root.join(&filename);
        let payload = serde_json::to_vec_pretty(&parsed_value).map_err(|err| {
            SoulBrowserError::internal(&format!("Failed to serialize structured output: {}", err))
        })?;
        fs::write(&artifact_path, payload).await.map_err(|err| {
            SoulBrowserError::internal(&format!("Failed to write structured artifact: {}", err))
        })?;
        OBSERVATION_CACHE.remove(&context.subject_id);

        let artifact_path_str = artifact_path.to_string_lossy().to_string();
        let mut metadata = serde_json::Map::new();
        metadata.insert("schema".into(), json!(schema));
        metadata.insert("artifact_path".into(), json!(artifact_path_str.clone()));

        let result = ToolExecutionResult {
            success: true,
            output: Some(json!({
                "status": "delivered",
                "schema": schema,
                "artifact_path": artifact_path_str,
            })),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
            metadata,
        };
        Ok(self.finish_tool(context, start, span, result))
    }

    fn run_data_parser(
        &self,
        tool_id: &str,
        observation: &Value,
        payload: &Value,
    ) -> Result<(String, Value), SoulBrowserError> {
        match tool_id {
            "data.parse.generic" => {
                let schema = payload
                    .get("schema")
                    .and_then(|v| v.as_str())
                    .unwrap_or("generic_observation_v1");
                Ok((
                    schema.to_string(),
                    parse_generic_observation(observation, schema),
                ))
            }
            "data.parse.market_info" => parse_market_info(observation)
                .map(|value| ("market_info_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse market info failed: {}", err))
                }),
            "data.parse.news_brief" => parse_news_brief(observation)
                .map(|value| ("news_brief_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse news brief failed: {}", err))
                }),
            "data.parse.twitter-feed" => parse_twitter_feed(observation)
                .map(|value| ("twitter_feed_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse twitter feed failed: {}", err))
                }),
            "data.parse.facebook-feed" => parse_facebook_feed(observation)
                .map(|value| ("facebook_feed_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse facebook feed failed: {}", err))
                }),
            "data.parse.hackernews-feed" => parse_hackernews_feed(observation)
                .map(|value| ("hackernews_feed_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse hackernews feed failed: {}", err))
                }),
            "data.parse.linkedin-profile" => parse_linkedin_profile(observation)
                .map(|value| ("linkedin_profile_v1".to_string(), value))
                .map_err(|err| {
                    SoulBrowserError::internal(&format!("Parse linkedin profile failed: {}", err))
                }),
            "data.parse.github-repo" => {
                let username = payload
                    .get("username")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if username.trim().is_empty() {
                    return Err(SoulBrowserError::validation_error(
                        "Missing field",
                        "'username' is required for data.parse.github-repo",
                    ));
                }
                parse_github_repos(observation, username)
                    .map(|value| ("github_repos_v1".to_string(), value))
                    .map_err(|err| {
                        SoulBrowserError::internal(&format!(
                            "Parse github repositories failed: {}",
                            err
                        ))
                    })
            }
            other => Err(SoulBrowserError::internal(&format!(
                "Unknown parse tool '{}'; supported tools must be registered",
                other
            ))),
        }
    }

    fn parse_anchor(value: &serde_json::Value) -> Result<AnchorDescriptor, SoulBrowserError> {
        if let Some(s) = value.as_str() {
            if s.trim().is_empty() {
                return Err(SoulBrowserError::validation_error(
                    "Invalid anchor",
                    "CSS selector cannot be empty",
                ));
            }
            return Ok(AnchorDescriptor::Css(s.to_string()));
        }

        let obj = value.as_object().ok_or_else(|| {
            SoulBrowserError::validation_error(
                "Invalid anchor",
                "Anchor must be a string or object",
            )
        })?;

        if let Some(selector) = obj.get("selector").and_then(|v| v.as_str()) {
            let strategy = obj
                .get("strategy")
                .or_else(|| obj.get("kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("css");
            return match strategy.to_ascii_lowercase().as_str() {
                "css" => Ok(AnchorDescriptor::Css(selector.to_string())),
                "text" => {
                    let exact = obj.get("exact").and_then(|v| v.as_bool()).unwrap_or(false);
                    Ok(AnchorDescriptor::Text {
                        content: selector.to_string(),
                        exact,
                    })
                }
                other => Err(SoulBrowserError::validation_error(
                    "Invalid anchor strategy",
                    &format!("Unsupported anchor strategy '{}'.", other),
                )),
            };
        }

        if let (Some(role), Some(name)) = (
            obj.get("role").and_then(|v| v.as_str()),
            obj.get("name").and_then(|v| v.as_str()),
        ) {
            if role.is_empty() || name.is_empty() {
                return Err(SoulBrowserError::validation_error(
                    "Invalid anchor",
                    "role/name cannot be empty",
                ));
            }
            return Ok(AnchorDescriptor::Aria {
                role: role.to_string(),
                name: name.to_string(),
            });
        }

        if let Some(content) = obj.get("text").and_then(|v| v.as_str()) {
            let exact = obj.get("exact").and_then(|v| v.as_bool()).unwrap_or(false);
            if content.is_empty() {
                return Err(SoulBrowserError::validation_error(
                    "Invalid anchor",
                    "Text anchor cannot be empty",
                ));
            }
            return Ok(AnchorDescriptor::Text {
                content: content.to_string(),
                exact,
            });
        }

        Err(SoulBrowserError::validation_error(
            "Invalid anchor",
            "Unrecognized anchor descriptor",
        ))
    }

    fn parse_scroll_behavior(
        value: Option<&serde_json::Value>,
    ) -> Result<ScrollBehavior, SoulBrowserError> {
        if let Some(val) = value {
            if let Some(kind) = val.as_str() {
                return match kind.to_ascii_lowercase().as_str() {
                    "smooth" => Ok(ScrollBehavior::Smooth),
                    "instant" => Ok(ScrollBehavior::Instant),
                    "natural" => Ok(ScrollBehavior::Smooth),
                    other => Err(SoulBrowserError::validation_error(
                        "Invalid scroll behavior",
                        &format!("Unsupported scroll behavior '{}'.", other),
                    )),
                };
            }
        }
        Ok(ScrollBehavior::default())
    }

    fn parse_scroll_target(value: &serde_json::Value) -> Result<ScrollTarget, SoulBrowserError> {
        if let Some(kind_value) = value.get("kind").and_then(|v| v.as_str()) {
            match kind_value.to_ascii_lowercase().as_str() {
                "top" => return Ok(ScrollTarget::Top),
                "bottom" => return Ok(ScrollTarget::Bottom),
                "pixels" | "delta" => {
                    let amount = value
                        .get("value")
                        .or_else(|| value.get("delta"))
                        .and_then(|v| v.as_i64())
                        .ok_or_else(|| {
                            SoulBrowserError::validation_error(
                                "Invalid scroll target",
                                "Delta scroll requires numeric value",
                            )
                        })?;
                    return Ok(ScrollTarget::Pixels(amount as i32));
                }
                "element" | "elementcenter" => {
                    let anchor_val = value.get("anchor").ok_or_else(|| {
                        SoulBrowserError::validation_error(
                            "Invalid scroll target",
                            "Element scroll requires anchor descriptor",
                        )
                    })?;
                    let anchor = Self::parse_anchor(anchor_val)?;
                    return Ok(ScrollTarget::Element(anchor));
                }
                "toy" => {
                    let y = value.get("value").and_then(|v| v.as_i64()).ok_or_else(|| {
                        SoulBrowserError::validation_error(
                            "Invalid scroll target",
                            "toY target requires numeric value",
                        )
                    })?;
                    return Ok(ScrollTarget::Pixels(y as i32));
                }
                other => {
                    return Err(SoulBrowserError::validation_error(
                        "Invalid scroll target",
                        &format!("Unsupported target kind '{}'.", other),
                    ))
                }
            }
        }

        if let Some(anchor_val) = value.get("anchor") {
            let anchor = Self::parse_anchor(anchor_val)?;
            return Ok(ScrollTarget::Element(anchor));
        }

        Err(SoulBrowserError::validation_error(
            "Invalid scroll target",
            "Target must specify kind or anchor",
        ))
    }

    fn parse_wait_condition_for_element(
        target: &serde_json::Value,
        condition: &serde_json::Value,
    ) -> Result<WaitCondition, SoulBrowserError> {
        let anchor_val = target.get("anchor").unwrap_or(target);
        let anchor = Self::parse_anchor(anchor_val)?;

        let kind = condition
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("visible");

        match kind.to_ascii_lowercase().as_str() {
            "visible" | "clickable" | "present" => Ok(WaitCondition::ElementVisible(anchor)),
            "hidden" | "removed" => Ok(WaitCondition::ElementHidden(anchor)),
            other => Err(SoulBrowserError::validation_error(
                "Unsupported condition",
                &format!(
                    "Condition '{}' is not supported yet. Supported: visible, clickable, present, hidden, removed.",
                    other
                ),
            )),
        }
    }

    fn parse_wait_condition_for_expect(
        expect: &serde_json::Value,
    ) -> Result<WaitCondition, SoulBrowserError> {
        if let Some(net) = expect.get("net") {
            if let Some(quiet_ms) = net.get("quiet_ms").and_then(|v| v.as_u64()) {
                return Ok(WaitCondition::NetworkIdle(quiet_ms));
            }
        }

        if let Some(duration_ms) = expect
            .get("duration_ms")
            .or_else(|| expect.get("sleep_ms"))
            .and_then(|v| v.as_u64())
        {
            return Ok(WaitCondition::Duration(duration_ms));
        }

        Err(SoulBrowserError::validation_error(
            "Unsupported expect",
            "wait-for-condition currently supports net.quiet_ms or duration_ms",
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for BrowserToolExecutor {
    async fn execute(
        &self,
        context: ToolExecutionContext,
    ) -> Result<ToolExecutionResult, SoulBrowserError> {
        let start = std::time::Instant::now();
        let span = obs_tracing::tool_span(&context.tool_id);
        let _span_guard = span.enter();
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "tool_id".to_string(),
            serde_json::Value::String(context.tool_id.clone()),
        );

        let exec_ctx = self.build_exec_ctx(context.route.as_ref(), context.timeout_ms);

        match context.tool_id.as_str() {
            "navigate-to-url" | "browser.navigate" => {
                let url = context
                    .input
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        SoulBrowserError::validation_error(
                            "Missing field",
                            "'url' is required for navigate-to-url",
                        )
                    })?;
                let wait = context
                    .input
                    .get("wait_tier")
                    .map(|v| Self::parse_wait_tier(Some(v)))
                    .unwrap_or(WaitTier::Idle);
                let report = self
                    .primitives
                    .navigate(&exec_ctx, url, wait)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        SoulBrowserError::internal(&format!("Navigate failed: {}", err))
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "navigated",
                        "url": url,
                        "latency_ms": report.latency_ms,
                        "wait_tier": format!("{:?}", wait).to_lowercase(),
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "click" | "browser.click" => {
                let anchor_val = context
                    .input
                    .get("anchor")
                    .or_else(|| context.input.get("selector"));
                let anchor_json = anchor_val.ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'anchor' or 'selector' is required for click",
                    )
                })?;
                let anchor = Self::parse_anchor(anchor_json)?;
                let wait = context
                    .input
                    .get("wait_tier")
                    .map(|v| Self::parse_wait_tier(Some(v)))
                    .unwrap_or(WaitTier::DomReady);
                let report = self
                    .primitives
                    .click(&exec_ctx, &anchor, wait)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        SoulBrowserError::internal(&format!("Click failed: {}", err))
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "clicked",
                        "anchor": anchor.to_string(),
                        "latency_ms": report.latency_ms,
                        "wait_tier": format!("{:?}", wait).to_lowercase(),
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "type-text" | "browser.type" => {
                let anchor_val = context
                    .input
                    .get("anchor")
                    .or_else(|| context.input.get("selector"));
                let anchor_json = anchor_val.ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'anchor' or 'selector' is required for type-text",
                    )
                })?;
                let anchor = Self::parse_anchor(anchor_json)?;
                let text = context
                    .input
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        SoulBrowserError::validation_error(
                            "Missing field",
                            "'text' is required for type-text",
                        )
                    })?;
                let submit = context
                    .input
                    .get("submit")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let wait_tier = context
                    .input
                    .get("wait_tier")
                    .map(|v| Self::parse_wait_tier(Some(v)));
                let report = self
                    .primitives
                    .type_text(&exec_ctx, &anchor, text, submit, wait_tier)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        SoulBrowserError::internal(&format!("Type failed: {}", err))
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "typed",
                        "anchor": anchor.to_string(),
                        "length": text.len(),
                        "submit": submit,
                        "latency_ms": report.latency_ms,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "select-option" | "browser.select" => {
                let anchor_val = context
                    .input
                    .get("anchor")
                    .or_else(|| context.input.get("selector"));
                let anchor_json = anchor_val.ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'anchor' or 'selector' is required for select-option",
                    )
                })?;
                let anchor = Self::parse_anchor(anchor_json)?;
                let value = context
                    .input
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        SoulBrowserError::validation_error(
                            "Missing field",
                            "'value' is required for select-option",
                        )
                    })?;
                let match_kind_value = context.input.get("match_kind").and_then(|v| v.as_str());
                let method = Self::parse_select_method(match_kind_value)?;
                let wait = context
                    .input
                    .get("wait_tier")
                    .map(|v| Self::parse_wait_tier(Some(v)))
                    .unwrap_or(WaitTier::DomReady);
                let report = self
                    .primitives
                    .select(&exec_ctx, &anchor, method, value, wait)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        SoulBrowserError::internal(&format!("Select failed: {}", err))
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "selected",
                        "anchor": anchor.to_string(),
                        "value": value,
                        "match_kind": match_kind_value,
                        "latency_ms": report.latency_ms,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "scroll-page" => {
                let target_json = context.input.get("target").ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'target' is required for scroll-page",
                    )
                })?;
                let target = Self::parse_scroll_target(target_json)?;
                let behavior = Self::parse_scroll_behavior(context.input.get("behavior"))?;
                let report = self
                    .primitives
                    .scroll(&exec_ctx, &target, behavior)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        SoulBrowserError::internal(&format!("Scroll failed: {}", err))
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "scrolled",
                        "target": format!("{:?}", target),
                        "behavior": format!("{:?}", behavior).to_lowercase(),
                        "latency_ms": report.latency_ms,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "wait-for-element" => {
                let target = context.input.get("target").ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'target' is required for wait-for-element",
                    )
                })?;
                let condition = context.input.get("condition").ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'condition' is required for wait-for-element",
                    )
                })?;
                let wait_condition = Self::parse_wait_condition_for_element(target, condition)?;
                let report = self
                    .primitives
                    .wait_for(&exec_ctx, &wait_condition, context.timeout_ms)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        SoulBrowserError::internal(&format!("Wait failed: {}", err))
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "condition_met",
                        "condition": format!("{:?}", wait_condition),
                        "latency_ms": report.latency_ms,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "wait-for-condition" => {
                let expect = context.input.get("expect").ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'expect' is required for wait-for-condition",
                    )
                })?;
                let wait_condition = Self::parse_wait_condition_for_expect(expect)?;
                let report = self
                    .primitives
                    .wait_for(&exec_ctx, &wait_condition, context.timeout_ms)
                    .await
                    .map_err(|err| {
                        self.record_error(&context, start, &span);
                        SoulBrowserError::internal(&format!("Wait failed: {}", err))
                    })?;
                let duration = report.latency_ms.max(start.elapsed().as_millis() as u64);
                let result = ToolExecutionResult {
                    success: report.ok,
                    output: Some(serde_json::json!({
                        "status": "condition_met",
                        "condition": format!("{:?}", wait_condition),
                        "latency_ms": report.latency_ms,
                    })),
                    error: None,
                    duration_ms: duration,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "get-element-info" => {
                let anchor_json = context.input.get("anchor").ok_or_else(|| {
                    SoulBrowserError::validation_error(
                        "Missing field",
                        "'anchor' is required for get-element-info",
                    )
                })?;
                let anchor = Self::parse_anchor(anchor_json)?;
                let include = context.input.get("include").cloned().unwrap_or_default();
                let result = ToolExecutionResult {
                    success: true,
                    output: Some(serde_json::json!({
                        "status": "inspected",
                        "anchor": anchor.to_string(),
                        "include": include,
                    })),
                    error: None,
                    duration_ms: start.elapsed().as_millis() as u64,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "retrieve-history" => {
                let limit = context
                    .input
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let result = ToolExecutionResult {
                    success: true,
                    output: Some(serde_json::json!({
                        "status": "history",
                        "events": serde_json::Value::Array(vec![]),
                        "limit": limit,
                    })),
                    error: None,
                    duration_ms: start.elapsed().as_millis() as u64,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "take-screenshot" | "browser.screenshot" => {
                let route = match context.route.as_ref() {
                    Some(route) => route,
                    None => {
                        let duration = start.elapsed().as_millis() as u64;
                        metadata.insert("reason".to_string(), serde_json::json!("missing route"));
                        let result = ToolExecutionResult {
                            success: false,
                            output: Some(serde_json::json!({
                                "status": "failed",
                                "reason": "execution route required",
                            })),
                            error: Some(
                                "take-screenshot requires a page/frame execution route".to_string(),
                            ),
                            duration_ms: duration,
                            metadata,
                        };
                        let result = self.finish_tool(&context, start, &span, result);
                        return Ok(result);
                    }
                };

                let page_id = match self.resolve_page_for_route(route).await {
                    Ok(id) => id,
                    Err(err) => {
                        let duration = start.elapsed().as_millis() as u64;
                        metadata.insert("reason".to_string(), serde_json::json!("invalid route"));
                        let result = ToolExecutionResult {
                            success: false,
                            output: Some(serde_json::json!({
                                "status": "failed",
                                "reason": err.dev_message().unwrap_or("invalid route"),
                            })),
                            error: Some(err.to_string()),
                            duration_ms: duration,
                            metadata,
                        };
                        let result = self.finish_tool(&context, start, &span, result);
                        return Ok(result);
                    }
                };

                if let Err(err) = self.ensure_adapter_started().await {
                    let duration = start.elapsed().as_millis() as u64;
                    metadata.insert(
                        "reason".to_string(),
                        serde_json::json!("adapter start failed"),
                    );
                    let result = ToolExecutionResult {
                        success: false,
                        output: Some(serde_json::json!({
                            "status": "failed",
                            "reason": err.dev_message().unwrap_or("adapter unavailable"),
                        })),
                        error: Some(err.to_string()),
                        duration_ms: duration,
                        metadata,
                    };
                    let result = self.finish_tool(&context, start, &span, result);
                    return Ok(result);
                }

                let remaining = exec_ctx.remaining_time();
                let deadline = if remaining.is_zero() {
                    Duration::from_secs(30)
                } else {
                    remaining
                };

                let session_id = route.session.0.clone();
                let page_id_str = route.page.0.clone();
                let frame_id = route.frame.0.clone();

                match self.adapter.screenshot(page_id, deadline).await {
                    Ok(bytes) => {
                        let duration = start.elapsed().as_millis() as u64;
                        let byte_len = bytes.len() as u64;
                        metadata.insert(
                            "page_id".to_string(),
                            serde_json::json!(page_id_str.clone()),
                        );
                        metadata.insert("byte_len".to_string(), serde_json::json!(byte_len));
                        metadata.insert("content_type".to_string(), serde_json::json!("image/png"));

                        let mut output = serde_json::json!({
                            "status": "captured",
                            "byte_len": byte_len,
                            "content_type": "image/png",
                            "route": {
                                "session": session_id,
                                "page": page_id_str,
                                "frame": frame_id,
                            },
                            "bytes": bytes,
                        });

                        if let Some(filename) =
                            context.input.get("filename").and_then(|v| v.as_str())
                        {
                            if let Some(obj) = output.as_object_mut() {
                                obj.insert("filename".to_string(), serde_json::json!(filename));
                            }
                        }

                        let result = ToolExecutionResult {
                            success: true,
                            output: Some(output),
                            error: None,
                            duration_ms: duration,
                            metadata,
                        };
                        let result = self.finish_tool(&context, start, &span, result);
                        Ok(result)
                    }
                    Err(err) => {
                        let duration = start.elapsed().as_millis() as u64;
                        let hint = err.hint.clone();
                        let message = err.to_string();
                        metadata.insert("adapter_error".to_string(), serde_json::json!(message));
                        let result = ToolExecutionResult {
                            success: false,
                            output: Some(serde_json::json!({
                                "status": "failed",
                                "reason": hint.unwrap_or_else(|| "cdp adapter error".to_string()),
                            })),
                            error: Some(format!("Screenshot failed: {}", err)),
                            duration_ms: duration,
                            metadata,
                        };
                        let result = self.finish_tool(&context, start, &span, result);
                        Ok(result)
                    }
                }
            }
            "complete-task" => {
                let task_id = context
                    .input
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        SoulBrowserError::validation_error(
                            "Missing field",
                            "'task_id' is required for complete-task",
                        )
                    })?;
                let outcome = context
                    .input
                    .get("outcome")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let summary = context
                    .input
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let result = ToolExecutionResult {
                    success: true,
                    output: Some(serde_json::json!({
                        "status": "recorded",
                        "task_id": task_id,
                        "outcome": outcome,
                        "summary_len": summary.len(),
                    })),
                    error: None,
                    duration_ms: start.elapsed().as_millis() as u64,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "report-insight" => {
                let insight = context
                    .input
                    .get("insight")
                    .or_else(|| context.input.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let result = ToolExecutionResult {
                    success: true,
                    output: Some(serde_json::json!({
                        "status": "recorded",
                        "insight_len": insight.len(),
                    })),
                    error: None,
                    duration_ms: start.elapsed().as_millis() as u64,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
            "data.extract-site" => {
                return self
                    .execute_observation_tool(&context, start, &span, exec_ctx)
                    .await;
            }
            tool if tool.starts_with("data.parse.") => {
                return self.execute_parse_tool(&context, start, &span, tool).await;
            }
            "data.deliver.structured" => {
                return self.execute_deliver_tool(&context, start, &span).await;
            }
            other => {
                let result = ToolExecutionResult {
                    success: false,
                    output: None,
                    error: Some(format!("Unknown tool: {}", other)),
                    duration_ms: start.elapsed().as_millis() as u64,
                    metadata,
                };
                let result = self.finish_tool(&context, start, &span, result);
                Ok(result)
            }
        }
    }

    fn cdp_adapter(&self) -> Option<Arc<CdpAdapter>> {
        Some(self.adapter.clone())
    }
}

fn parse_generic_observation(observation: &Value, schema: &str) -> Value {
    let data = observation.get("data").unwrap_or(observation);
    let url = data.get("url").and_then(|v| v.as_str());
    let title = data.get("title").and_then(|v| v.as_str());
    let text_sample = data
        .get("text_sample")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let summary: String = text_sample.chars().take(480).collect();

    json!({
        "schema": schema,
        "source_url": url,
        "captured_at": data.get("fetched_at").and_then(|v| v.as_str()),
        "title": title,
        "summary": summary,
        "text_sample": text_sample,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cdp_adapter::{
        event_bus, AdapterError, AdapterErrorKind, CdpAdapter, CdpConfig, CdpTransport,
        CommandTarget, PageId as AdapterPageId, SessionId as AdapterSessionId, TransportEvent,
    };
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::{Mutex, OnceCell};
    use uuid::Uuid;

    #[tokio::test]
    async fn test_tool_manager() {
        let manager = BrowserToolManager::new("test-tenant".to_string());

        // Register all default tools (including legacy aliases)
        manager.register_default_tools().await.unwrap();

        // List tools
        let tools = manager.list_tools(None).await.unwrap();
        assert!(tools.len() >= 12);

        // Get specific tool
        let tool = manager.get_tool("navigate-to-url").await.unwrap();
        assert!(tool.is_some());

        // Execute tool
        let result = manager
            .execute(
                "navigate-to-url",
                "test-user",
                serde_json::json!({"url": "https://example.com"}),
            )
            .await
            .unwrap();
        assert!(result["success"].as_bool().unwrap());
        assert_eq!(
            result["metadata"]["tool_id"].as_str().unwrap(),
            "navigate-to-url"
        );
    }

    #[tokio::test]
    async fn test_tool_executor() {
        let executor = BrowserToolExecutor::new();

        let context = ToolExecutionContext {
            tool_id: "navigate-to-url".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({"url": "https://example.com"}),
            timeout_ms: 5000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: None,
        };

        let result = executor.execute(context).await.unwrap();
        assert!(result.success);
        assert!(result.output.is_some());

        let select_ctx = ToolExecutionContext {
            tool_id: "select-option".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({
                "selector": "#country",
                "value": "us",
                "match_kind": "value"
            }),
            timeout_ms: 5000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: None,
        };

        let select_result = executor.execute(select_ctx).await.unwrap();
        assert!(select_result.success);
        assert!(select_result.output.is_some());
    }

    #[tokio::test]
    async fn test_take_screenshot_uses_adapter_bytes() {
        #[derive(Clone)]
        struct ScreenshotTransport {
            data: Arc<Mutex<Vec<(CommandTarget, String)>>>,
            screenshot_b64: String,
        }

        #[async_trait]
        impl CdpTransport for ScreenshotTransport {
            async fn start(&self) -> Result<(), AdapterError> {
                Ok(())
            }

            async fn next_event(&self) -> Option<TransportEvent> {
                None
            }

            async fn send_command(
                &self,
                target: CommandTarget,
                method: &str,
                _params: serde_json::Value,
            ) -> Result<serde_json::Value, AdapterError> {
                self.data
                    .lock()
                    .await
                    .push((target.clone(), method.to_string()));

                match method {
                    "Target.setDiscoverTargets" | "Target.setAutoAttach" => Ok(json!({})),
                    "Page.captureScreenshot" => Ok(json!({ "data": self.screenshot_b64 })),
                    other => Err(AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint(format!("unexpected method: {}", other))),
                }
            }
        }

        let (bus, _rx) = event_bus(8);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transport = ScreenshotTransport {
            data: calls.clone(),
            screenshot_b64: "iVBORw0KGgo=".to_string(),
        };

        let adapter = Arc::new(CdpAdapter::with_transport(
            CdpConfig::default(),
            bus,
            Arc::new(transport),
        ));

        let executor = BrowserToolExecutor {
            adapter: adapter.clone(),
            primitives: Arc::new(DefaultActionPrimitives::new(
                adapter.clone(),
                Arc::new(DefaultWaitStrategy::default()),
            )),
            policy_view: Arc::new(PolicyView::from(default_snapshot())),
            adapter_ready: OnceCell::new(),
            route_pages: DashMap::new(),
            page_routes: DashMap::new(),
            pending_pages: DashMap::new(),
        };

        let page_uuid = Uuid::new_v4();
        let session_uuid = Uuid::new_v4();
        adapter.register_page(
            AdapterPageId(page_uuid),
            AdapterSessionId(session_uuid),
            None,
            Some("mock-session".to_string()),
        );

        let frame_uuid = Uuid::new_v4().to_string();
        let route = ExecRoute::new(
            SessionId(session_uuid.to_string()),
            PageId(page_uuid.to_string()),
            FrameId(frame_uuid.clone()),
        );

        let context = ToolExecutionContext {
            tool_id: "take-screenshot".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({"filename": "mock.png"}),
            timeout_ms: 5_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: Some(route),
        };

        let result = executor.execute(context).await.unwrap();
        assert!(result.success, "expected screenshot tool to succeed");
        let output = result.output.expect("missing output payload");
        assert_eq!(output["byte_len"].as_u64(), Some(8));
        let bytes = output["bytes"].as_array().expect("bytes array");
        assert_eq!(bytes.len(), 8);
        assert_eq!(output["filename"].as_str(), Some("mock.png"));
        {
            let recorded = calls.lock().await;
            assert!(recorded
                .iter()
                .any(|(_, method)| method == "Page.captureScreenshot"));
        }
        assert_eq!(executor.route_pages.len(), 1);
        assert!(executor.pending_pages.is_empty());

        let second_route = ExecRoute::new(
            SessionId(session_uuid.to_string()),
            PageId(page_uuid.to_string()),
            FrameId(frame_uuid),
        );
        let second_context = ToolExecutionContext {
            tool_id: "take-screenshot".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({"filename": "second.png"}),
            timeout_ms: 5_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: Some(second_route),
        };

        let second_result = executor.execute(second_context).await.unwrap();
        assert!(second_result.success);

        let recorded = calls.lock().await;
        let create_count = recorded
            .iter()
            .filter(|(_, method)| method == "Target.createTarget")
            .count();
        assert_eq!(create_count, 0, "noop transport should not create targets");
        assert_eq!(
            executor.route_pages.len(),
            1,
            "cached route should be reused"
        );
        assert!(
            executor.pending_pages.is_empty(),
            "pending map should be clear"
        );
    }

    #[tokio::test]
    async fn test_scroll_tool_succeeds() {
        let executor = BrowserToolExecutor::new();

        let context = ToolExecutionContext {
            tool_id: "scroll-page".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({
                "target": { "kind": "top" },
                "behavior": "instant"
            }),
            timeout_ms: 2_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: None,
        };

        let result = executor.execute(context).await.unwrap();
        assert!(result.success);
        let output = result.output.expect("missing scroll output");
        assert_eq!(output["status"].as_str(), Some("scrolled"));
    }

    #[tokio::test]
    async fn test_wait_tools_produce_reports() {
        let executor = BrowserToolExecutor::new();

        let element_wait = ToolExecutionContext {
            tool_id: "wait-for-element".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({
                "target": { "anchor": "#app" },
                "condition": { "kind": "visible" },
                "timeout_ms": 500,
            }),
            timeout_ms: 2_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: None,
        };

        let element_result = executor.execute(element_wait).await.unwrap();
        assert!(element_result.success);
        assert_eq!(
            element_result.output.as_ref().unwrap()["status"].as_str(),
            Some("condition_met")
        );

        let condition_wait = ToolExecutionContext {
            tool_id: "wait-for-condition".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({
                "expect": { "duration_ms": 100 },
                "timeout_ms": 500,
            }),
            timeout_ms: 1_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: None,
        };

        let condition_result = executor.execute(condition_wait).await.unwrap();
        assert!(condition_result.success);
        assert_eq!(
            condition_result.output.as_ref().unwrap()["status"].as_str(),
            Some("condition_met")
        );
    }

    #[tokio::test]
    async fn test_retrieve_history_returns_payload() {
        let executor = BrowserToolExecutor::new();

        let context = ToolExecutionContext {
            tool_id: "retrieve-history".to_string(),
            tenant_id: "test".to_string(),
            subject_id: "user-1".to_string(),
            input: serde_json::json!({ "limit": 3 }),
            timeout_ms: 1_000,
            trace_id: uuid::Uuid::new_v4().to_string(),
            route: None,
        };

        let result = executor.execute(context).await.unwrap();
        assert!(result.success);
        let output = result.output.expect("missing retrieve-history output");
        assert_eq!(output["status"].as_str(), Some("history"));
        assert_eq!(output["limit"].as_u64(), Some(3));
    }
}
