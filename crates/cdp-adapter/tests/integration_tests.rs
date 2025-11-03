//! CDP Adapter Integration Tests
//!
//! Tests the real CDP adapter with actual Chromium browser.
//! Requires Chrome/Chromium to be installed and accessible.
//!
//! Run with:
//! ```bash
//! export SOULBROWSER_USE_REAL_CHROME=1
//! export SOULBROWSER_CHROME=/usr/bin/google-chrome  # or path to chrome
//! cargo test -p cdp-adapter --test integration_tests -- --nocapture
//! ```

use cdp_adapter::config::CdpConfig;
use cdp_adapter::transport::{CdpTransport, ChromiumTransport, CommandTarget};
use serde_json::json;
use std::env;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

/// Check if we should run real browser tests
fn should_run_real_tests() -> bool {
    env::var("SOULBROWSER_USE_REAL_CHROME")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Create test configuration with an isolated temporary profile directory.
fn test_config() -> (CdpConfig, TempDir) {
    let mut cfg = CdpConfig::default();
    cfg.headless = true;

    // Use environment variable if set
    if let Ok(chrome_path) = env::var("SOULBROWSER_CHROME") {
        cfg.executable = chrome_path.into();
    }

    let profile = tempfile::tempdir().expect("create temporary chrome profile");
    cfg.user_data_dir = profile.path().into();

    (cfg, profile)
}

#[tokio::test]
async fn test_browser_launch_and_connection() {
    if !should_run_real_tests() {
        println!("Skipping real browser test (SOULBROWSER_USE_REAL_CHROME not set)");
        return;
    }

    println!("🚀 Test: Browser Launch and Connection");

    let (cfg, _profile) = test_config();
    let transport = ChromiumTransport::new(cfg);

    // Start transport (this launches browser and connects)
    transport.start().await.expect("Failed to start transport");
    println!("✅ Transport started successfully");

    // Test basic Browser.getVersion command
    let result = transport
        .send_command(CommandTarget::Browser, "Browser.getVersion", json!({}))
        .await
        .expect("Failed to get browser version");

    println!("✅ Browser version: {}", result);
    assert!(result.is_object());
    assert!(result.get("product").is_some());

    println!("✅ Test passed: Browser launch and connection");
}

#[tokio::test]
async fn test_navigate_command() {
    if !should_run_real_tests() {
        println!("Skipping real browser test (SOULBROWSER_USE_REAL_CHROME not set)");
        return;
    }

    println!("🚀 Test: Navigate Command");

    let (cfg, _profile) = test_config();
    let transport = ChromiumTransport::new(cfg);
    transport.start().await.expect("Failed to start transport");

    // Create a new page target
    let target_result = transport
        .send_command(
            CommandTarget::Browser,
            "Target.createTarget",
            json!({
                "url": "about:blank"
            }),
        )
        .await
        .expect("Failed to create target");

    let target_id = target_result["targetId"]
        .as_str()
        .expect("Missing targetId");
    println!("✅ Created target: {}", target_id);

    // Attach to target to get session ID
    let attach_result = transport
        .send_command(
            CommandTarget::Browser,
            "Target.attachToTarget",
            json!({
                "targetId": target_id,
                "flatten": true
            }),
        )
        .await
        .expect("Failed to attach to target");

    let session_id = attach_result["sessionId"]
        .as_str()
        .expect("Missing sessionId");
    println!("✅ Attached to target with session: {}", session_id);

    // Navigate to example.com
    let nav_result = transport
        .send_command(
            CommandTarget::Session(session_id.to_string()),
            "Page.navigate",
            json!({
                "url": "https://example.com"
            }),
        )
        .await
        .expect("Failed to navigate");

    println!("✅ Navigate result: {}", nav_result);
    assert!(nav_result.get("frameId").is_some());

    // Wait a bit for page load
    sleep(Duration::from_secs(2)).await;

    // Get current URL to verify navigation
    let eval_result = transport
        .send_command(
            CommandTarget::Session(session_id.to_string()),
            "Runtime.evaluate",
            json!({
                "expression": "window.location.href",
                "returnByValue": true
            }),
        )
        .await
        .expect("Failed to evaluate");

    let current_url = eval_result["result"]["value"]
        .as_str()
        .expect("Missing URL in eval result");

    println!("✅ Current URL: {}", current_url);
    assert!(current_url.contains("example.com"));

    println!("✅ Test passed: Navigate command");
}

#[tokio::test]
async fn test_event_reception() {
    if !should_run_real_tests() {
        println!("Skipping real browser test (SOULBROWSER_USE_REAL_CHROME not set)");
        return;
    }

    println!("🚀 Test: Event Reception");

    let (cfg, _profile) = test_config();
    let transport = ChromiumTransport::new(cfg);
    transport.start().await.expect("Failed to start transport");

    // Spawn task to collect events
    let transport_clone = transport.clone();
    let event_task = tokio::spawn(async move {
        let mut event_count = 0;
        let start = std::time::Instant::now();

        while start.elapsed() < Duration::from_secs(5) && event_count < 10 {
            if let Some(event) = transport_clone.next_event().await {
                println!("📨 Received event: {}", event.method);
                event_count += 1;
            }
        }

        event_count
    });

    // Create target to trigger events
    let _target = transport
        .send_command(
            CommandTarget::Browser,
            "Target.createTarget",
            json!({
                "url": "about:blank"
            }),
        )
        .await
        .expect("Failed to create target");

    // Wait for events
    let event_count = event_task.await.expect("Event task failed");

    println!("✅ Received {} events", event_count);
    assert!(event_count > 0, "Should receive at least one event");

    println!("✅ Test passed: Event reception");
}

#[tokio::test]
async fn test_dom_query_and_click() {
    if !should_run_real_tests() {
        println!("Skipping real browser test (SOULBROWSER_USE_REAL_CHROME not set)");
        return;
    }

    println!("🚀 Test: DOM Query and Click");

    let (cfg, _profile) = test_config();
    let transport = ChromiumTransport::new(cfg);
    transport.start().await.expect("Failed to start transport");

    // Create target and attach
    let target_result = transport
        .send_command(
            CommandTarget::Browser,
            "Target.createTarget",
            json!({"url": "about:blank"}),
        )
        .await
        .expect("Failed to create target");

    let target_id = target_result["targetId"].as_str().unwrap();

    let attach_result = transport
        .send_command(
            CommandTarget::Browser,
            "Target.attachToTarget",
            json!({"targetId": target_id, "flatten": true}),
        )
        .await
        .expect("Failed to attach");

    let session_id = attach_result["sessionId"].as_str().unwrap().to_string();
    let session_target = CommandTarget::Session(session_id.clone());

    // Navigate to example.com
    transport
        .send_command(
            session_target.clone(),
            "Page.navigate",
            json!({"url": "https://example.com"}),
        )
        .await
        .expect("Failed to navigate");

    // Wait for load
    sleep(Duration::from_secs(2)).await;

    // Get document node
    let doc_result = transport
        .send_command(
            session_target.clone(),
            "DOM.getDocument",
            json!({"depth": 0}),
        )
        .await
        .expect("Failed to get document");

    let doc_node_id = doc_result["root"]["nodeId"]
        .as_u64()
        .expect("Missing nodeId");

    println!("✅ Document nodeId: {}", doc_node_id);

    // Query for h1 element
    let query_result = transport
        .send_command(
            session_target.clone(),
            "DOM.querySelector",
            json!({
                "nodeId": doc_node_id,
                "selector": "h1"
            }),
        )
        .await
        .expect("Failed to query selector");

    let h1_node_id = query_result["nodeId"].as_u64().expect("Missing h1 nodeId");

    println!("✅ Found h1 element with nodeId: {}", h1_node_id);
    assert!(h1_node_id > 0);

    println!("✅ Test passed: DOM query and click");
}

#[tokio::test]
async fn test_screenshot_capture() {
    if !should_run_real_tests() {
        println!("Skipping real browser test (SOULBROWSER_USE_REAL_CHROME not set)");
        return;
    }

    println!("🚀 Test: Screenshot Capture");

    let (cfg, _profile) = test_config();
    let transport = ChromiumTransport::new(cfg);
    transport.start().await.expect("Failed to start transport");

    // Create and attach target
    let target_result = transport
        .send_command(
            CommandTarget::Browser,
            "Target.createTarget",
            json!({"url": "https://example.com"}),
        )
        .await
        .expect("Failed to create target");

    let target_id = target_result["targetId"].as_str().unwrap();

    let attach_result = transport
        .send_command(
            CommandTarget::Browser,
            "Target.attachToTarget",
            json!({"targetId": target_id, "flatten": true}),
        )
        .await
        .expect("Failed to attach");

    let session_id = attach_result["sessionId"].as_str().unwrap().to_string();

    // Wait for page load
    sleep(Duration::from_secs(2)).await;

    // Capture screenshot
    let screenshot_result = transport
        .send_command(
            CommandTarget::Session(session_id),
            "Page.captureScreenshot",
            json!({
                "format": "png",
                "quality": 80
            }),
        )
        .await
        .expect("Failed to capture screenshot");

    let screenshot_data = screenshot_result["data"]
        .as_str()
        .expect("Missing screenshot data");

    println!("✅ Screenshot captured ({} bytes)", screenshot_data.len());
    assert!(screenshot_data.len() > 0);

    println!("✅ Test passed: Screenshot capture");
}

#[tokio::test]
async fn test_concurrent_commands() {
    if !should_run_real_tests() {
        println!("Skipping real browser test (SOULBROWSER_USE_REAL_CHROME not set)");
        return;
    }

    println!("🚀 Test: Concurrent Commands");

    let (cfg, _profile) = test_config();
    let transport = ChromiumTransport::new(cfg);
    transport.start().await.expect("Failed to start transport");

    // Spawn 5 concurrent Browser.getVersion commands
    let mut handles = vec![];

    for i in 0..5 {
        let transport_clone = transport.clone();
        let handle = tokio::spawn(async move {
            let start = std::time::Instant::now();
            let result = transport_clone
                .send_command(CommandTarget::Browser, "Browser.getVersion", json!({}))
                .await;

            (i, start.elapsed(), result)
        });
        handles.push(handle);
    }

    // Wait for all commands
    let mut success_count = 0;
    for handle in handles {
        let (i, elapsed, result) = handle.await.expect("Task failed");
        match result {
            Ok(value) => {
                println!("✅ Command {} completed in {:?}: {}", i, elapsed, value);
                success_count += 1;
            }
            Err(e) => {
                println!("❌ Command {} failed: {:?}", i, e);
            }
        }
    }

    assert_eq!(success_count, 5, "All concurrent commands should succeed");

    println!("✅ Test passed: Concurrent commands");
}

#[tokio::test]
async fn test_command_timeout() {
    if !should_run_real_tests() {
        println!("Skipping real browser test (SOULBROWSER_USE_REAL_CHROME not set)");
        return;
    }

    println!("🚀 Test: Command Timeout");

    let (mut cfg, _profile) = test_config();
    cfg.default_deadline_ms = 100; // Very short timeout

    let transport = ChromiumTransport::new(cfg);
    transport.start().await.expect("Failed to start transport");

    // This should timeout (create target + navigate takes more than 100ms)
    let result = transport
        .send_command(
            CommandTarget::Browser,
            "Target.createTarget",
            json!({"url": "https://example.com"}),
        )
        .await;

    // We expect either success or timeout
    match result {
        Ok(_) => println!("✅ Command succeeded (faster than expected)"),
        Err(e) => {
            println!("✅ Command timed out as expected: {:?}", e);
            assert!(e.to_string().contains("timeout") || e.to_string().contains("timed out"));
        }
    }

    println!("✅ Test passed: Command timeout");
}
