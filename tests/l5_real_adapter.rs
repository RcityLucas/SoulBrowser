use std::time::Duration;

use serde_json::json;
use serial_test::serial;
use soulbrowser_cli::tools::{BrowserToolExecutor, ToolExecutionContext, ToolExecutor};
use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId};
use tempfile::{Builder, TempDir};
use tokio::time::timeout;
use uuid::Uuid;

struct ProfileGuard {
    temp: Option<TempDir>,
    previous: Option<String>,
}

impl ProfileGuard {
    fn new() -> Self {
        match std::env::var("SOULBROWSER_CHROME_PROFILE") {
            Ok(existing) => ProfileGuard {
                temp: None,
                previous: Some(existing),
            },
            Err(_) => {
                let dir = Builder::new()
                    .prefix("soulbrowser-profile-")
                    .tempdir()
                    .expect("create temporary chrome profile directory");
                std::env::set_var("SOULBROWSER_CHROME_PROFILE", dir.path());
                ProfileGuard {
                    temp: Some(dir),
                    previous: None,
                }
            }
        }
    }
}

impl Drop for ProfileGuard {
    fn drop(&mut self) {
        if self.temp.take().is_some() {
            std::env::remove_var("SOULBROWSER_CHROME_PROFILE");
        } else if let Some(value) = &self.previous {
            std::env::set_var("SOULBROWSER_CHROME_PROFILE", value);
        }
    }
}

fn is_real_adapter_enabled() -> bool {
    matches!(
        std::env::var("SOULBROWSER_USE_REAL_CHROME")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[tokio::test]
#[serial]
async fn l5_real_adapter_screenshot_smoke() -> Result<(), Box<dyn std::error::Error>> {
    if !is_real_adapter_enabled() {
        println!("Skipping real-adapter smoke test (set SOULBROWSER_USE_REAL_CHROME=1 to enable)");
        return Ok(());
    }

    let _profile = ProfileGuard::new();
    let executor = BrowserToolExecutor::new();
    let adapter = executor
        .cdp_adapter()
        .expect("executor should expose cdp adapter");

    let exec_route = ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new());

    let screenshot_context = ToolExecutionContext {
        tool_id: "take-screenshot".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({ "filename": "integration.png" }),
        timeout_ms: 15_000,
        trace_id: Uuid::new_v4().to_string(),
        route: Some(exec_route),
    };

    let result = timeout(
        Duration::from_secs(30),
        executor.execute(screenshot_context),
    )
    .await??;
    assert!(result.success, "screenshot via real adapter should succeed");

    adapter.shutdown().await;
    Ok(())
}

#[tokio::test]
#[serial]
async fn l5_real_adapter_toolchain() -> Result<(), Box<dyn std::error::Error>> {
    if !is_real_adapter_enabled() {
        println!(
            "Skipping real-adapter toolchain test (set SOULBROWSER_USE_REAL_CHROME=1 to enable)"
        );
        return Ok(());
    }

    let _profile = ProfileGuard::new();
    let executor = BrowserToolExecutor::new();
    let adapter = executor
        .cdp_adapter()
        .expect("executor should expose cdp adapter");

    let session_id = SessionId::new();
    let page_id = PageId::new();
    let frame_id = FrameId::new();

    let common_route = ExecRoute::new(session_id.clone(), page_id.clone(), frame_id.clone());

    let nav_context = ToolExecutionContext {
        tool_id: "navigate-to-url".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({
            "url": "https://httpbin.org/forms/post",
            "wait_tier": "idle"
        }),
        timeout_ms: 30_000,
        trace_id: Uuid::new_v4().to_string(),
        route: Some(common_route.clone()),
    };
    let nav_result = timeout(Duration::from_secs(60), executor.execute(nav_context)).await??;
    assert!(nav_result.success, "navigate-to-url should succeed");

    let wait_context = ToolExecutionContext {
        tool_id: "wait-for-element".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({
            "target": { "anchor": { "strategy": "css", "selector": "input[name='custname']" } },
            "condition": { "kind": "visible" },
            "timeout_ms": 5_000
        }),
        timeout_ms: 10_000,
        trace_id: Uuid::new_v4().to_string(),
        route: Some(common_route.clone()),
    };
    let wait_result = timeout(Duration::from_secs(20), executor.execute(wait_context)).await??;
    assert!(wait_result.success, "wait-for-element should succeed");

    let type_context = ToolExecutionContext {
        tool_id: "type-text".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({
            "selector": "input[name='custname']",
            "text": "SoulBrowser",
            "submit": false
        }),
        timeout_ms: 10_000,
        trace_id: Uuid::new_v4().to_string(),
        route: Some(common_route.clone()),
    };
    let type_result = timeout(Duration::from_secs(20), executor.execute(type_context)).await??;
    assert!(type_result.success, "type-text should succeed");

    let select_context = ToolExecutionContext {
        tool_id: "select-option".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({
            "selector": "select[name='size']",
            "value": "large",
            "match_kind": "value",
            "wait_tier": "domready"
        }),
        timeout_ms: 10_000,
        trace_id: Uuid::new_v4().to_string(),
        route: Some(common_route.clone()),
    };
    let select_result =
        timeout(Duration::from_secs(20), executor.execute(select_context)).await??;
    assert!(select_result.success, "select-option should succeed");

    let click_context = ToolExecutionContext {
        tool_id: "click".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({
            "selector": "input[type='submit']",
            "wait_tier": "domready"
        }),
        timeout_ms: 10_000,
        trace_id: Uuid::new_v4().to_string(),
        route: Some(common_route.clone()),
    };
    let click_result = timeout(Duration::from_secs(20), executor.execute(click_context)).await??;
    assert!(click_result.success, "click should succeed");

    let wait_condition_context = ToolExecutionContext {
        tool_id: "wait-for-condition".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({
            "expect": { "duration_ms": 500 },
            "timeout_ms": 2_000
        }),
        timeout_ms: 5_000,
        trace_id: Uuid::new_v4().to_string(),
        route: Some(common_route.clone()),
    };
    let wait_condition_result = timeout(
        Duration::from_secs(10),
        executor.execute(wait_condition_context),
    )
    .await??;
    assert!(
        wait_condition_result.success,
        "wait-for-condition should succeed"
    );

    let scroll_context = ToolExecutionContext {
        tool_id: "scroll-page".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({
            "target": { "kind": "bottom" },
            "behavior": "instant"
        }),
        timeout_ms: 10_000,
        trace_id: Uuid::new_v4().to_string(),
        route: Some(common_route.clone()),
    };
    let scroll_result =
        timeout(Duration::from_secs(20), executor.execute(scroll_context)).await??;
    assert!(scroll_result.success, "scroll-page should succeed");

    let info_context = ToolExecutionContext {
        tool_id: "get-element-info".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({
            "anchor": { "strategy": "css", "selector": "h1" },
            "include": { "attributes": true }
        }),
        timeout_ms: 5_000,
        trace_id: Uuid::new_v4().to_string(),
        route: Some(common_route.clone()),
    };
    let info_result = timeout(Duration::from_secs(10), executor.execute(info_context)).await??;
    assert!(info_result.success, "get-element-info should succeed");

    let history_context = ToolExecutionContext {
        tool_id: "retrieve-history".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({ "limit": 5 }),
        timeout_ms: 5_000,
        trace_id: Uuid::new_v4().to_string(),
        route: None,
    };
    let history_result =
        timeout(Duration::from_secs(10), executor.execute(history_context)).await??;
    assert!(history_result.success, "retrieve-history should succeed");

    let complete_context = ToolExecutionContext {
        tool_id: "complete-task".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({
            "task_id": "l5-real-adapter",
            "outcome": "success",
            "summary": "end-to-end toolchain completed"
        }),
        timeout_ms: 2_000,
        trace_id: Uuid::new_v4().to_string(),
        route: None,
    };
    let complete_result = executor.execute(complete_context).await?;
    assert!(complete_result.success, "complete-task should succeed");

    let insight_context = ToolExecutionContext {
        tool_id: "report-insight".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({
            "insight": "All 12 L5 tools executed against real CDP"
        }),
        timeout_ms: 2_000,
        trace_id: Uuid::new_v4().to_string(),
        route: None,
    };
    let insight_result = executor.execute(insight_context).await?;
    assert!(insight_result.success, "report-insight should succeed");

    let screenshot_context = ToolExecutionContext {
        tool_id: "take-screenshot".to_string(),
        tenant_id: "integration".to_string(),
        subject_id: "real-adapter".to_string(),
        input: json!({ "filename": "toolchain.png" }),
        timeout_ms: 20_000,
        trace_id: Uuid::new_v4().to_string(),
        route: Some(common_route),
    };
    let screenshot_result = timeout(
        Duration::from_secs(30),
        executor.execute(screenshot_context),
    )
    .await??;
    assert!(screenshot_result.success, "take-screenshot should succeed");

    adapter.shutdown().await;
    Ok(())
}
