//! Example demonstrating Soul Base integration with SoulBrowser
//!
//! Shows how soul-base components enhance browser automation with:
//! - Authentication and authorization
//! - Interceptor chains for processing
//! - Storage and persistence
//! - Observability and monitoring

// Since this is a binary crate, we need to include the modules directly
// In a real project, these would be exposed through a lib.rs
include!("../src/soul_integration/mod.rs");
include!("../src/soul_integration/types.rs");
include!("../src/soul_integration/interceptors.rs");
include!("../src/soul_integration/auth.rs");
include!("../src/soul_integration/storage.rs");
include!("../src/soul_integration/observability.rs");

use soul_integration::{
    ActionResult, ActionType, BrowserAction, SoulIntegration, SoulIntegrationBuilder,
};
use std::time::Duration;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🌟 SoulBrowser with Soul Base Integration Demo\n");

    // 1. Build Soul Integration with specific components
    println!("1️⃣ Initializing Soul Integration...");
    let integration = SoulIntegrationBuilder::new()
        .with_auth(true)
        .with_interceptors(true)
        .with_observability(true)
        .with_storage(true)
        .with_tenant("demo-tenant".to_string())
        .build();

    println!("   ✅ Soul Integration initialized with all components\n");

    // 2. Demonstrate Authentication
    println!("2️⃣ Testing Authentication...");

    if let Some(auth_manager) = integration.auth() {
        // Try with demo token
        let demo_input = AuthInput::Token("demo-token".to_string());
        match auth_manager.authenticate(&demo_input).await {
            Ok(subject) => {
                println!("   ✅ Authenticated as: {:?}", subject.id);
            }
            Err(e) => {
                println!("   ❌ Authentication failed: {}", e);
            }
        }

        // Try with invalid token
        let invalid_input = AuthInput::Token("invalid-token".to_string());
        match auth_manager.authenticate(&invalid_input).await {
            Ok(_) => {
                println!("   ❌ Should not authenticate with invalid token");
            }
            Err(_) => {
                println!("   ✅ Correctly rejected invalid token");
            }
        }
    }

    println!();

    // 3. Demonstrate Interceptor Chain
    println!("3️⃣ Testing Interceptor Chain...");

    // Create a test action
    let navigate_action = BrowserAction {
        id: "demo-1".to_string(),
        action_type: ActionType::Navigate,
        target: Some("https://example.com".to_string()),
        value: None,
        metadata: Some(serde_json::json!({
            "demo": true,
            "source": "example"
        })),
    };

    // Process through Soul pipeline
    println!("   Processing Navigate action through Soul pipeline...");
    match integration.process_action(navigate_action.clone()).await {
        Ok(result) => {
            println!("   ✅ Action processed successfully");
            println!("      - Duration: {}ms", result.duration_ms);
            println!("      - Success: {}", result.success);
        }
        Err(e) => {
            println!("   ❌ Action processing failed: {}", e);
        }
    }

    println!();

    // 4. Demonstrate Policy-based Authorization
    println!("4️⃣ Testing Policy-based Authorization...");

    // Add a custom policy interceptor
    if let Some(interceptor_chain) = integration.interceptors() {
        // Create a policy that blocks certain domains
        let mut policy_interceptor = PolicyInterceptor::new();
        policy_interceptor.add_policy(Policy {
            name: "block-dangerous-sites".to_string(),
            rule: PolicyRule::DenyDomain("dangerous.com".to_string()),
            effect: PolicyEffect::Deny,
        });

        // Test with blocked domain
        let blocked_action = BrowserAction {
            id: "demo-2".to_string(),
            action_type: ActionType::Navigate,
            target: Some("https://dangerous.com/malware".to_string()),
            value: None,
            metadata: None,
        };

        println!("   Testing navigation to blocked domain...");
        match integration.process_action(blocked_action).await {
            Ok(_) => {
                println!("   ❌ Should have blocked dangerous site");
            }
            Err(e) => {
                println!("   ✅ Correctly blocked: {}", e);
            }
        }
    }

    println!();

    // 5. Demonstrate Complex Workflow
    println!("5️⃣ Testing Complex Workflow with Multiple Actions...");

    let workflow = vec![
        BrowserAction {
            id: "workflow-1".to_string(),
            action_type: ActionType::Navigate,
            target: Some("https://example.com/login".to_string()),
            value: None,
            metadata: None,
        },
        BrowserAction {
            id: "workflow-2".to_string(),
            action_type: ActionType::Type,
            target: Some("#username".to_string()),
            value: Some("demo@example.com".to_string()),
            metadata: None,
        },
        BrowserAction {
            id: "workflow-3".to_string(),
            action_type: ActionType::Type,
            target: Some("#password".to_string()),
            value: Some("secure_password".to_string()),
            metadata: None,
        },
        BrowserAction {
            id: "workflow-4".to_string(),
            action_type: ActionType::Click,
            target: Some("#login-button".to_string()),
            value: None,
            metadata: None,
        },
        BrowserAction {
            id: "workflow-5".to_string(),
            action_type: ActionType::Wait,
            target: None,
            value: Some("2000".to_string()), // Wait 2 seconds
            metadata: None,
        },
        BrowserAction {
            id: "workflow-6".to_string(),
            action_type: ActionType::Screenshot,
            target: None,
            value: Some("dashboard.png".to_string()),
            metadata: None,
        },
    ];

    println!("   Executing workflow with {} actions...", workflow.len());

    for (i, action) in workflow.into_iter().enumerate() {
        println!("   Step {}: {:?}", i + 1, action.action_type);

        match integration.process_action(action).await {
            Ok(result) => {
                println!("      ✅ Success ({}ms)", result.duration_ms);
            }
            Err(e) => {
                println!("      ❌ Failed: {}", e);
                break;
            }
        }

        // Small delay between actions
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    println!();

    // 6. Demonstrate Storage and Event Query
    println!("6️⃣ Testing Storage and Event Query...");

    if let Some(storage) = integration.storage() {
        use soulbrowser_cli::soul_integration::storage::EventQuery;

        // Query events from this session
        let query = EventQuery::new()
            .with_action_type("Navigate".to_string())
            .limit(5);

        match storage.query_events(query).await {
            Ok(events) => {
                println!("   Found {} navigation events", events.len());
                for event in events.iter().take(3) {
                    println!(
                        "      - Action: {} at {:?}",
                        event.action_id, event.timestamp
                    );
                }
            }
            Err(e) => {
                println!("   ❌ Failed to query events: {}", e);
            }
        }
    }

    println!();

    // 7. Demonstrate Observability Export
    println!("7️⃣ Testing Observability Export...");

    if let Some(observer) = integration.observer() {
        match observer.export().await {
            Ok(_) => {
                println!("   ✅ Observability data exported successfully");
            }
            Err(e) => {
                println!("   ❌ Failed to export observability data: {}", e);
            }
        }
    }

    println!();

    // 8. Demonstrate Rate Limiting
    println!("8️⃣ Testing Rate Limiting...");

    println!("   Sending rapid actions to test rate limiting...");
    for i in 1..=70 {
        let action = BrowserAction {
            id: format!("rapid-{}", i),
            action_type: ActionType::Click,
            target: Some(format!("#button-{}", i)),
            value: None,
            metadata: None,
        };

        match integration.process_action(action).await {
            Ok(_) => {
                if i % 10 == 0 {
                    println!("      ✅ Action {} processed", i);
                }
            }
            Err(e) => {
                println!("      ⏱️ Rate limited at action {}: {}", i, e);
                break;
            }
        }
    }

    println!();

    // 9. Summary
    println!("📊 Soul Integration Demo Summary:");
    println!("   • Authentication: Working with token-based auth");
    println!("   • Interceptors: Logging, validation, rate limiting, retry, cache");
    println!("   • Policies: Domain-based access control");
    println!("   • Storage: Event persistence and querying");
    println!("   • Observability: Metrics, traces, and logs");
    println!("   • Rate Limiting: 60 actions per minute protection");

    println!("\n✨ Demo completed successfully!");

    Ok(())
}
