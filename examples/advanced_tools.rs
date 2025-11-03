//! Advanced tools example for SoulBrowser
//!
//! Run with: cargo run --example advanced_tools

use anyhow::Result;
use l3_locator::{LocatorConfig, SmartLocator};
use l3_postcondition::{GateConfig, PostConditionGate};
use l4_event_store::{EventStore, EventStoreConfig};
use l5_tools::{ToolBuilder, ToolContext, ToolParams, ToolRegistry};
use soulbrowser::{Soul, SoulConfig};
use std::sync::Arc;
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("🌟 SoulBrowser Advanced Tools Example");
    println!("======================================\n");

    // Create Soul with custom config
    let config = SoulConfig::default();
    let soul = Soul::with_config(config).await?;

    // Create a session
    let session = soul.create_session().await?;

    // Setup tool context
    let event_store = Arc::new(EventStore::new(EventStoreConfig::default()).await?);
    let locator = SmartLocator::new(LocatorConfig::default());
    let post_condition_gate = PostConditionGate::new(GateConfig::default());

    let context = ToolContext {
        cdp_session: session.cdp_session.clone(),
        locator,
        post_condition_gate,
        event_store: event_store.clone(),
        execution_context: l3_primitives::ExecutionContext {
            cdp_session: session.cdp_session.clone(),
            state_center: session.state_center.clone(),
            policy_engine: Arc::new(l1_policy::PolicyEngine::new(Default::default())),
            retry_policy: l3_primitives::RetryPolicy::default(),
            timeout_policy: l3_primitives::TimeoutPolicy::default(),
            recovery_strategy: l3_primitives::RecoveryStrategy::default(),
        },
    };

    // Create tool registry
    let registry = ToolRegistry::new();

    println!("Available tools:");
    for tool in registry.list() {
        println!("  - {}", tool);
    }
    println!();

    // Example 1: Navigate to URL using tools
    println!("1. Navigating to example.com...");
    let navigate_result = ToolBuilder::new("navigate-to-url")
        .params(serde_json::json!({
            "url": "https://example.com",
            "wait_tier": "idle"
        }))
        .with_screenshot()
        .execute(&registry, &context)
        .await?;

    println!("   Navigation success: {}", navigate_result.success);
    if let Some(artifacts) = &navigate_result.artifacts {
        println!("   Captured {} artifacts", artifacts.len());
    }

    // Example 2: Wait for element
    println!("\n2. Waiting for h1 element...");
    let wait_result = ToolBuilder::new("wait-for-element")
        .params(serde_json::json!({
            "target": {
                "anchor": {
                    "strategy": "css",
                    "selector": "h1"
                }
            },
            "condition": {
                "kind": "present"
            },
            "continuous_ms": 150,
            "timeout_ms": 5000
        }))
        .execute(&registry, &context)
        .await?;

    println!("   Wait success: {}", wait_result.success);

    // Example 3: Take screenshot
    println!("\n3. Taking screenshot...");
    let screenshot_result = ToolBuilder::new("take-screenshot")
        .params(serde_json::json!({
            "options": {
                "full_page": false,
                "format": "Png"
            }
        }))
        .execute(&registry, &context)
        .await?;

    println!("   Screenshot success: {}", screenshot_result.success);

    // Query stored events
    println!("\n4. Querying stored events...");
    let events = event_store
        .query(l4_event_store::QueryBuilder::new().limit(10).build())
        .await?;

    println!("   Found {} events in store", events.len());
    for event in events.iter().take(3) {
        println!(
            "   - Event {}: {:?} at {}",
            event.id,
            event.event_type,
            event.timestamp.format("%H:%M:%S")
        );
    }

    // Cleanup
    session.close().await?;
    soul.shutdown().await?;

    println!("\n✨ Advanced tools example completed!");

    Ok(())
}
