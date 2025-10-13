#![cfg(feature = "legacy-tests")]

//! Integration test for soul-base components

use soulbrowser_cli::auth::{BrowserAuthManager, SessionManager};
use soulbrowser_cli::browser_impl::{BrowserConfig, L0Protocol, L1BrowserManager};
use soulbrowser_cli::storage::{BrowserEvent, QueryParams, StorageManager};
use soulbrowser_cli::types::BrowserType;

#[tokio::test]
async fn test_soul_base_browser_launch() {
    // Test that we can initialize and launch browser with soul-base
    let l0 = L0Protocol::new()
        .await
        .expect("Should initialize L0Protocol with soulbase-config");

    let browser_config = BrowserConfig {
        browser_type: BrowserType::Chromium,
        headless: true,
        window_size: Some((1280, 720)),
        devtools: false,
    };

    let mut l1 = L1BrowserManager::new(l0, browser_config)
        .await
        .expect("Should initialize L1BrowserManager with soul-base auth and storage");

    let browser = l1
        .launch_browser()
        .await
        .expect("Should launch browser with soul-base components");

    let _page = browser
        .new_page()
        .await
        .expect("Should create page with interceptor chain");

    // Verify that page was created successfully
    assert!(true, "Browser and page created with soul-base components");
}

#[tokio::test]
async fn test_soul_base_storage() {
    use soulbase_types::tenant::TenantId;

    // Create in-memory storage for testing
    let storage = StorageManager::in_memory();

    // Create a test event
    let event = BrowserEvent {
        id: "test-event-1".to_string(),
        tenant: TenantId("test-tenant".to_string()),
        session_id: "test-session".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        event_type: "test".to_string(),
        data: serde_json::json!({"action": "test"}),
        sequence: 1,
        tags: vec!["test".to_string()],
    };

    // Store the event
    storage
        .backend()
        .store_event(event.clone())
        .await
        .expect("Should store event successfully");

    // Query the event back
    let query = QueryParams {
        session_id: Some("test-session".to_string()),
        event_type: None,
        from_timestamp: None,
        to_timestamp: None,
        limit: 10,
        offset: 0,
    };

    let events = storage
        .backend()
        .query_events(query)
        .await
        .expect("Should query events successfully");

    // Assert we got our event back
    assert_eq!(events.len(), 1, "Should retrieve exactly one event");
    assert_eq!(events[0].id, "test-event-1", "Event ID should match");
    assert_eq!(
        events[0].session_id, "test-session",
        "Session ID should match"
    );
}

#[tokio::test]
async fn test_soul_base_auth() {
    use soulbase_types::subject::SubjectKind;

    // Create auth manager
    let auth_manager = BrowserAuthManager::new("test-tenant".to_string())
        .await
        .expect("Should create auth manager");

    // Authenticate with token
    let auth_session = auth_manager
        .authenticate_token("demo-user".to_string())
        .await
        .expect("Should authenticate with demo token");

    // Verify subject properties
    let subject = auth_session.subject();
    assert_eq!(subject.kind, SubjectKind::User, "Should be a user subject");
    assert_eq!(subject.tenant.0, "test-tenant", "Tenant should match");

    // Test authorization (default policy allows the standard routes)
    let decision = auth_manager
        .authorize_request(
            &auth_session,
            "POST",
            "browser://session/navigate",
            "session-demo",
            Some(&serde_json::json!({ "url": "https://example.com" })),
        )
        .await
        .expect("Should receive authorization decision");
    assert!(decision.allow);

    // Test session management
    let session_manager = SessionManager::new();
    let session_id = session_manager.create_session(auth_session.clone()).await;

    assert!(!session_id.is_empty(), "Session ID should not be empty");

    // Verify we can retrieve the session
    let retrieved = session_manager.get_session(&session_id).await;
    assert!(
        retrieved.is_some(),
        "Should be able to retrieve created session"
    );
    assert_eq!(
        retrieved.unwrap().subject().subject_id.0,
        subject.subject_id.0,
        "Retrieved session should match the subject"
    );
}

#[tokio::test]
async fn test_soul_base_tools() {
    use soulbrowser_cli::tools::BrowserToolManager;

    // Create tool manager
    let tool_manager = BrowserToolManager::new("test-tenant".to_string());

    // Register default tools
    tool_manager
        .register_default_tools()
        .await
        .expect("Should register default tools");

    // List available tools
    let tools = tool_manager
        .list_tools(None)
        .await
        .expect("Should list tools");

    // Verify we have the expected tools
    assert!(tools.len() >= 4, "Should have at least 4 browser tools");

    let tool_ids: Vec<String> = tools.iter().map(|t| t.manifest.id.0.clone()).collect();
    assert!(
        tool_ids.contains(&"browser.navigate".to_string()),
        "Should have navigate tool"
    );
    assert!(
        tool_ids.contains(&"browser.click".to_string()),
        "Should have click tool"
    );
    assert!(
        tool_ids.contains(&"browser.type".to_string()),
        "Should have type tool"
    );
    assert!(
        tool_ids.contains(&"browser.screenshot".to_string()),
        "Should have screenshot tool"
    );

    // Test tool execution
    let result = tool_manager
        .execute(
            "browser.navigate",
            "demo-user",
            serde_json::json!({"url": "https://example.com"}),
        )
        .await
        .expect("Should execute navigate tool");

    // Verify execution result
    assert!(
        result["success"].as_bool().unwrap_or(false),
        "Tool execution should succeed"
    );
    assert_eq!(
        result["metadata"]["tool_id"].as_str().unwrap(),
        "browser.navigate",
        "Tool ID should match"
    );
}
