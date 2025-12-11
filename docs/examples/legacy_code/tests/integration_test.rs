#[cfg(feature = "full-stack")]
mod full_stack {

    //! Integration tests for SoulBrowser

    use anyhow::Result;
    use l3_locator::{LocatorConfig, SmartLocator};
    use l3_postcondition::{conditions::presets, GateConfig, PostConditionGate};
    use l3_primitives::{ExecutionContext, PrimitiveAction, RetryPolicy, TimeoutPolicy};
    use l4_event_store::{EventStore, EventStoreConfig, QueryBuilder};
    use l5_tools::{ToolBuilder, ToolRegistry};
    use soulbrowser::{BrowserOptions, Soul, SoulConfig};
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Test basic Soul creation and lifecycle
    #[tokio::test]
    async fn test_soul_lifecycle() -> Result<()> {
        // Create Soul instance
        let soul = Soul::new().await?;

        // Create a session
        let session = soul.create_session().await?;
        assert!(!session.id().to_string().is_empty());

        // Get all sessions
        let sessions = soul.get_sessions().await;
        assert_eq!(sessions.len(), 1);

        // Close session
        let session_id = session.id().clone();
        soul.close_session(&session_id).await?;

        // Verify session removed
        let sessions = soul.get_sessions().await;
        assert_eq!(sessions.len(), 0);

        // Shutdown
        soul.shutdown().await?;

        Ok(())
    }

    /// Test browser creation with options
    #[tokio::test]
    async fn test_browser_options() -> Result<()> {
        let soul = Soul::new().await?;

        let options = BrowserOptions {
            headless: true,
            width: 1024,
            height: 768,
            devtools: false,
            ..Default::default()
        };

        let browser = soulbrowser::Browser::with_options(&soul, options).await?;
        assert!(browser.main_session().is_some());

        soul.shutdown().await?;
        Ok(())
    }

    /// Test L3 primitives execution
    #[tokio::test]
    async fn test_primitives() -> Result<()> {
        use l3_primitives::{DefaultPrimitiveExecutor, PrimitiveExecutor};

        let soul = Soul::new().await?;
        let session = soul.create_session().await?;

        let context = ExecutionContext {
            cdp_session: session.cdp_session.clone(),
            state_center: session.state_center.clone(),
            policy_engine: Arc::new(l1_policy::PolicyEngine::new(Default::default())),
            retry_policy: RetryPolicy::default(),
            timeout_policy: TimeoutPolicy::default(),
            recovery_strategy: l3_primitives::RecoveryStrategy::default(),
        };

        let executor = DefaultPrimitiveExecutor;

        // Test navigation
        let action = PrimitiveAction::Navigate {
            url: "data:text/html,<h1>Test Page</h1>".to_string(),
        };

        let result = executor.execute(&action, &context).await?;
        assert!(result.success);

        soul.shutdown().await?;
        Ok(())
    }

    /// Test smart locator with healing
    #[tokio::test]
    async fn test_smart_locator() -> Result<()> {
        let mut locator = SmartLocator::new(LocatorConfig::default());

        // Test CSS locator creation
        let css_locator = SmartLocator::from_css("#test-id");
        assert_eq!(css_locator.selector, "#test-id");
        assert_eq!(css_locator.locator_type, l3_locator::LocatorType::Css);

        // Test text locator
        let text_locator = SmartLocator::from_text("Submit");
        assert_eq!(text_locator.selector, "Submit");
        assert_eq!(text_locator.locator_type, l3_locator::LocatorType::Text);

        // Test ARIA locator
        let aria_locator = SmartLocator::from_aria("button", Some("Save"));
        assert!(aria_locator.selector.contains("role='button'"));
        assert!(aria_locator.selector.contains("aria-label='Save'"));

        Ok(())
    }

    /// Test post-condition validation
    #[tokio::test]
    async fn test_postconditions() -> Result<()> {
        let mut gate = PostConditionGate::new(GateConfig::default());

        // Test condition set creation
        let mut conditions = l3_postcondition::conditions::ConditionSet::new("Test");
        conditions.element_exists("#button", true);
        conditions.text_contains("#message", "Success", false);
        conditions.no_console_errors(false);

        assert_eq!(conditions.conditions.len(), 3);

        // Test preset conditions
        let nav_conditions = presets::navigation_success(Some("/home"));
        assert!(!nav_conditions.conditions.is_empty());

        let form_conditions = presets::form_submission_success();
        assert!(!form_conditions.conditions.is_empty());

        Ok(())
    }

    /// Test event store operations
    #[tokio::test]
    async fn test_event_store() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let config = EventStoreConfig {
            storage_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let store = EventStore::new(config).await?;

        // Create and store an event
        let event = l4_event_store::Event {
            id: "test-1".to_string(),
            session_id: "session-1".to_string(),
            timestamp: chrono::Utc::now(),
            event_type: l4_event_store::EventType::Action,
            data: l4_event_store::EventData::Action {
                action_type: "click".to_string(),
                target: Some("#button".to_string()),
                parameters: serde_json::json!({}),
            },
            sequence: 1,
            parent_id: None,
            tags: vec!["test".to_string()],
        };

        store.store(event.clone()).await?;

        // Query events
        let query = QueryBuilder::new()
            .session_id("session-1")
            .tag("test")
            .limit(10)
            .build();

        let events = store.query(query).await?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "test-1");

        // Get specific event
        let retrieved = store.get("test-1").await?;
        assert_eq!(retrieved.session_id, "session-1");

        Ok(())
    }

    /// Test L5 tools registry
    #[tokio::test]
    async fn test_tools_registry() -> Result<()> {
        let registry = ToolRegistry::new();

        // Check that default tools are registered
        let tools = registry.list();
        assert!(tools.contains(&"navigate-to-url".to_string()));
        assert!(tools.contains(&"click".to_string()));
        assert!(tools.contains(&"type-text".to_string()));
        assert!(tools.contains(&"wait-for-element".to_string()));
        assert!(tools.contains(&"take-screenshot".to_string()));

        // Test tool builder
        let builder = ToolBuilder::new("click")
            .params(serde_json::json!({
                "selector": "#button",
                "options": {
                    "button": "Left",
                    "click_count": 1
                }
            }))
            .timeout(std::time::Duration::from_secs(5))
            .with_screenshot();

        // Verify builder configuration
        assert!(builder.params.screenshot);
        assert!(builder.params.timeout.is_some());

        Ok(())
    }

    /// Test retry policies
    #[tokio::test]
    async fn test_retry_policies() -> Result<()> {
        use l3_primitives::retry::{RetryPolicy, RetryStrategy};
        use std::time::Duration;

        // Test exponential backoff
        let policy = RetryPolicy::exponential(3, Duration::from_millis(100));
        assert_eq!(policy.max_retries(), 3);

        // Test delay calculation (without jitter for predictable testing)
        let mut policy = policy;
        policy.jitter = false;

        assert_eq!(policy.delay_for_retry(0), Duration::from_millis(100));
        assert_eq!(policy.delay_for_retry(1), Duration::from_millis(200));
        assert_eq!(policy.delay_for_retry(2), Duration::from_millis(400));

        // Test fixed delay
        let fixed_policy = RetryPolicy::fixed(5, Duration::from_secs(1));
        assert_eq!(fixed_policy.max_retries(), 5);

        // Test immediate retry
        let immediate_policy = RetryPolicy::immediate(3);
        assert_eq!(
            immediate_policy.delay_for_retry(0),
            Duration::from_millis(0)
        );

        Ok(())
    }

    /// Test timeout policies
    #[tokio::test]
    async fn test_timeout_policies() -> Result<()> {
        use l3_primitives::timeout::TimeoutPolicy;
        use std::time::Duration;

        // Test default timeout
        let policy = TimeoutPolicy::default();

        let nav_action = PrimitiveAction::Navigate {
            url: "https://example.com".to_string(),
        };
        assert_eq!(
            policy.timeout_for_action(&nav_action),
            Duration::from_secs(30)
        );

        let click_action = PrimitiveAction::Click {
            selector: "#button".to_string(),
        };
        assert_eq!(
            policy.timeout_for_action(&click_action),
            Duration::from_secs(10)
        );

        // Test strict policy
        let strict_policy = TimeoutPolicy::strict();
        assert_eq!(strict_policy.navigation_timeout, Duration::from_secs(10));
        assert_eq!(strict_policy.interaction_timeout, Duration::from_secs(3));

        // Test relaxed policy
        let relaxed_policy = TimeoutPolicy::relaxed();
        assert_eq!(relaxed_policy.navigation_timeout, Duration::from_secs(60));
        assert_eq!(relaxed_policy.interaction_timeout, Duration::from_secs(30));

        Ok(())
    }

    /// Test recovery strategies
    #[tokio::test]
    async fn test_recovery_strategies() -> Result<()> {
        use l3_primitives::recovery::{RecoveryPlan, RecoveryStrategy};

        let strategy = RecoveryStrategy::default();

        // Test recovery for element not found
        let action = PrimitiveAction::Click {
            selector: "#button".to_string(),
        };
        let error = anyhow::anyhow!("Element not found");

        let plan = strategy.create_plan(&action, &error);
        match plan {
            RecoveryPlan::AlternateSelector {
                original,
                alternatives,
            } => {
                assert_eq!(original, "#button");
                assert!(!alternatives.is_empty());
            }
            _ => panic!("Expected AlternateSelector recovery plan"),
        }

        // Test recovery for not visible
        let error = anyhow::anyhow!("Element not visible");
        let plan = strategy.create_plan(&action, &error);
        match plan {
            RecoveryPlan::ScrollIntoView { selector } => {
                assert_eq!(selector, "#button");
            }
            _ => panic!("Expected ScrollIntoView recovery plan"),
        }

        Ok(())
    }
}
