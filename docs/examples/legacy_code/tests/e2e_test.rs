#[cfg(feature = "full-stack")]
mod full_stack {
    //! End-to-end tests for SoulBrowser

    use anyhow::Result;
    use l3_flow::{FlowBuilder, FlowExecutor};
    use l3_primitives::{ExecutionContext, PrimitiveAction};
    use l4_event_store::{EventStore, EventStoreConfig, QueryBuilder};
    use l4_snapshot_store::{CaptureOptions, SnapshotConfig, SnapshotStore};
    use l5_tools::{ToolBuilder, ToolRegistry};
    use l6_metrics::{MetricsCenter, MetricsConfig, StandardMetrics};
    use l6_timeline::{EntryType, Severity, TimelineConfig, TimelineManager};
    use soulbrowser::{BrowserOptions, Soul};
    use std::time::Duration;
    use tempfile::TempDir;

    /// Test complete user flow
    #[tokio::test]
    async fn test_complete_user_flow() -> Result<()> {
        // Setup
        let temp_dir = TempDir::new()?;
        let soul = Soul::new().await?;

        // Create browser session
        let options = BrowserOptions {
            headless: true,
            width: 1280,
            height: 720,
            ..Default::default()
        };

        let browser = soulbrowser::Browser::with_options(&soul, options).await?;
        assert!(browser.main_session().is_some());

        // Setup event store
        let event_config = EventStoreConfig {
            storage_dir: temp_dir.path().join("events"),
            ..Default::default()
        };
        let event_store = EventStore::new(event_config).await?;

        // Setup snapshot store
        let snapshot_config = SnapshotConfig {
            storage_dir: temp_dir.path().join("snapshots"),
            ..Default::default()
        };
        let snapshot_store = SnapshotStore::new(snapshot_config).await?;

        // Setup metrics
        let metrics_config = MetricsConfig::default();
        let metrics = MetricsCenter::new(metrics_config).await?;
        metrics.start().await;

        // Setup timeline
        let timeline_config = TimelineConfig::default();
        let timeline = TimelineManager::new(timeline_config).await?;

        // Create timeline for session
        let session_id = browser.main_session().unwrap().id().to_string();
        timeline.create_timeline(&session_id).await?;

        // Simulate user flow
        timeline
            .add_milestone(&session_id, "Flow Started", None)
            .await?;

        // Record metrics
        metrics.inc_counter(StandardMetrics::NAVIGATION_COUNT, 1.0);
        metrics.observe_histogram(StandardMetrics::PAGE_LOAD_TIME, 1.5);

        // Take snapshot
        if let Some(session) = browser.main_session() {
            snapshot_store
                .capture(&session_id, &session.cdp_session, CaptureOptions::default())
                .await?;
        }

        // Complete timeline
        timeline
            .add_milestone(&session_id, "Flow Completed", None)
            .await?;
        timeline.complete_timeline(&session_id).await?;

        // Analyze timeline
        let analysis = timeline.analyze_timeline(&session_id).await?;
        assert!(!analysis.sections.is_empty());

        // Get metrics
        let all_metrics = metrics.get_all();
        assert!(all_metrics.contains_key(StandardMetrics::NAVIGATION_COUNT));

        // Query events
        let query = QueryBuilder::new()
            .session_id(&session_id)
            .limit(10)
            .build();
        let events = event_store.query(query).await?;

        // Cleanup
        browser.close().await?;
        soul.shutdown().await?;

        Ok(())
    }

    /// Test flow orchestration
    #[tokio::test]
    async fn test_flow_orchestration() -> Result<()> {
        let soul = Soul::new().await?;
        let session = soul.create_session().await?;

        // Build a flow
        let flow = FlowBuilder::new("Test Flow")
            .description("E2E test flow")
            .action(
                "nav",
                "Navigate",
                PrimitiveAction::Navigate {
                    url: "data:text/html,<h1>Test</h1>".to_string(),
                },
            )
            .wait(
                "wait",
                "Wait",
                l3_flow::WaitCondition::Duration(Duration::from_millis(100)),
            )
            .build();

        // Validate flow
        flow.validate()?;

        // Execute flow
        let executor = FlowExecutor::new();
        let context = ExecutionContext {
            cdp_session: session.cdp_session.clone(),
            state_center: session.state_center.clone(),
            policy_engine: std::sync::Arc::new(l1_policy::PolicyEngine::new(Default::default())),
            retry_policy: l3_primitives::RetryPolicy::default(),
            timeout_policy: l3_primitives::TimeoutPolicy::default(),
            recovery_strategy: l3_primitives::RecoveryStrategy::default(),
        };

        let result = executor.execute(flow, &context).await?;
        assert!(result.success);

        soul.shutdown().await?;
        Ok(())
    }

    /// Test tool execution
    #[tokio::test]
    async fn test_tool_execution() -> Result<()> {
        let soul = Soul::new().await?;
        let session = soul.create_session().await?;

        // Setup tool context
        let event_store = std::sync::Arc::new(EventStore::new(EventStoreConfig::default()).await?);

        let context = l5_tools::ToolContext {
            cdp_session: session.cdp_session.clone(),
            locator: l3_locator::SmartLocator::new(Default::default()),
            post_condition_gate: l3_postcondition::PostConditionGate::new(Default::default()),
            event_store: event_store.clone(),
            execution_context: l3_primitives::ExecutionContext {
                cdp_session: session.cdp_session.clone(),
                state_center: session.state_center.clone(),
                policy_engine: std::sync::Arc::new(
                    l1_policy::PolicyEngine::new(Default::default()),
                ),
                retry_policy: l3_primitives::RetryPolicy::default(),
                timeout_policy: l3_primitives::TimeoutPolicy::default(),
                recovery_strategy: l3_primitives::RecoveryStrategy::default(),
            },
        };

        let registry = ToolRegistry::new();

        // Test navigation tool
        let nav_result = ToolBuilder::new("navigate-to-url")
            .params(serde_json::json!({
                "url": "data:text/html,<h1>Tool Test</h1>",
                "wait_tier": "idle"
            }))
            .timeout(Duration::from_secs(5))
            .execute(&registry, &context)
            .await?;

        assert!(nav_result.success);

        soul.shutdown().await?;
        Ok(())
    }

    /// Test snapshot and replay
    #[tokio::test]
    async fn test_snapshot_replay() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let soul = Soul::new().await?;
        let session = soul.create_session().await?;
        let session_id = session.id().to_string();

        // Setup snapshot store
        let config = SnapshotConfig {
            storage_dir: temp_dir.path().to_path_buf(),
            include_screenshots: false, // Disable for test
            ..Default::default()
        };
        let store = SnapshotStore::new(config).await?;

        // Capture snapshot
        let snapshot1 = store
            .capture(
                &session_id,
                &session.cdp_session,
                CaptureOptions {
                    include_screenshot: false,
                    ..Default::default()
                },
            )
            .await?;

        // Wait and capture another
        tokio::time::sleep(Duration::from_millis(100)).await;

        let snapshot2 = store
            .capture(
                &session_id,
                &session.cdp_session,
                CaptureOptions {
                    include_screenshot: false,
                    ..Default::default()
                },
            )
            .await?;

        // Compare snapshots
        let diff = store.diff(&snapshot1.id, &snapshot2.id).await?;
        assert!(diff.similarity >= 0.0);

        // Query snapshots
        let query = l4_snapshot_store::SnapshotQuery::builder()
            .session_id(&session_id)
            .build();

        let snapshots = store.query(query).await?;
        assert_eq!(snapshots.len(), 2);

        soul.shutdown().await?;
        Ok(())
    }

    /// Test metrics collection and analysis
    #[tokio::test]
    async fn test_metrics_analysis() -> Result<()> {
        let config = MetricsConfig {
            collection_interval: Duration::from_millis(100),
            enable_anomaly_detection: true,
            enable_trend_analysis: true,
            ..Default::default()
        };

        let metrics = MetricsCenter::new(config).await?;

        // Record various metrics
        for i in 0..10 {
            metrics.inc_counter("test_counter", 1.0);
            metrics.set_gauge("test_gauge", i as f64);
            metrics.observe_histogram("test_histogram", (i as f64) * 0.1);

            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Check metrics
        let all = metrics.get_all();
        assert!(all.contains_key("test_counter"));
        assert!(all.contains_key("test_gauge"));
        assert!(all.contains_key("test_histogram"));

        // Get history (if available)
        if let Some(history) = metrics.get_history("test_gauge").await {
            assert!(!history.points.is_empty());
        }

        Ok(())
    }

    /// Test timeline recording and visualization
    #[tokio::test]
    async fn test_timeline_visualization() -> Result<()> {
        use l6_timeline::{EntryData, TimelineEntry};

        let config = TimelineConfig::default();
        let manager = TimelineManager::new(config).await?;

        let session_id = "test-session";
        let timeline = manager.create_timeline(session_id).await?;

        // Add various entries
        for i in 0..5 {
            let entry = TimelineEntry {
                id: uuid::Uuid::new_v4().to_string(),
                timestamp: chrono::Utc::now(),
                entry_type: EntryType::Navigation,
                duration: Some(chrono::Duration::seconds(i)),
                data: EntryData::Navigation {
                    url: format!("https://example.com/page{}", i),
                    status_code: Some(200),
                    load_time: Some(chrono::Duration::seconds(i)),
                },
                tags: vec!["test".to_string()],
                severity: Severity::Info,
            };

            manager.add_entry(session_id, entry).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Add milestone
        manager
            .add_milestone(
                session_id,
                "Test Milestone",
                Some("Test milestone description".to_string()),
            )
            .await?;

        // Create and end segment
        let segment_id = manager
            .start_segment(session_id, "Test Segment", l6_timeline::SegmentType::Test)
            .await?;

        tokio::time::sleep(Duration::from_millis(200)).await;
        manager.end_segment(session_id, &segment_id).await?;

        // Analyze timeline
        let analysis = manager.analyze_timeline(session_id).await?;
        assert_eq!(analysis.summary.total_entries, 5);

        // Visualize timeline
        let html = manager
            .visualize_timeline(session_id, l6_timeline::VisualizationFormat::Html)
            .await?;
        assert!(html.contains("test-session"));

        // Get correlations
        let correlations = manager.get_correlations(session_id).await?;

        // Complete timeline
        manager.complete_timeline(session_id).await?;

        // Export timeline
        let json_export = manager
            .export_timeline(session_id, l6_timeline::ExportFormat::Json)
            .await?;
        assert!(!json_export.is_empty());

        Ok(())
    }

    /// Test error handling and recovery
    #[tokio::test]
    async fn test_error_recovery() -> Result<()> {
        use l3_primitives::{RecoveryPlan, RecoveryStrategy};

        let strategy = RecoveryStrategy::default();

        // Test element not found recovery
        let action = PrimitiveAction::Click {
            selector: "#missing-button".to_string(),
        };
        let error = anyhow::anyhow!("Element not found");

        let plan = strategy.create_plan(&action, &error);
        assert!(matches!(plan, RecoveryPlan::AlternateSelector { .. }));

        // Test element not visible recovery
        let error2 = anyhow::anyhow!("Element not visible");
        let plan2 = strategy.create_plan(&action, &error2);
        assert!(matches!(plan2, RecoveryPlan::ScrollIntoView { .. }));

        Ok(())
    }

    /// Test locator healing
    #[tokio::test]
    async fn test_locator_healing() -> Result<()> {
        use l3_locator::{HealingStrategy, LocatorConfig, SmartLocator};

        let config = LocatorConfig {
            enable_healing: true,
            healing_strategy: HealingStrategy::Adaptive,
            ..Default::default()
        };

        let mut locator = SmartLocator::new(config);

        // Test healing history
        locator
            .record_success("#old-selector", "#new-selector")
            .await;

        // Test alternative generation
        let alternatives = locator.generate_alternatives("#button");
        assert!(!alternatives.is_empty());
        assert!(alternatives.contains(&"button".to_string()));

        Ok(())
    }

    /// Test post-conditions validation
    #[tokio::test]
    async fn test_postconditions() -> Result<()> {
        use l3_postcondition::{conditions::ConditionSet, GateConfig, PostConditionGate};

        let gate = PostConditionGate::new(GateConfig::default());

        // Create condition set
        let mut conditions = ConditionSet::new("Test Conditions");
        conditions.element_exists("#content", true);
        conditions.no_console_errors(false);
        conditions.response_received("/api/data", true);

        assert_eq!(conditions.conditions.len(), 3);

        // Test presets
        let nav_conditions = l3_postcondition::conditions::presets::navigation_success(None);
        assert!(!nav_conditions.conditions.is_empty());

        Ok(())
    }

    /// Test concurrent operations
    #[tokio::test]
    async fn test_concurrent_operations() -> Result<()> {
        let soul = Soul::new().await?;

        // Create multiple sessions concurrently
        let mut handles = Vec::new();

        for i in 0..3 {
            let soul_clone = soul.clone();
            let handle = tokio::spawn(async move {
                let session = soul_clone.create_session().await?;
                tokio::time::sleep(Duration::from_millis(100)).await;
                soul_clone.close_session(session.id()).await?;
                Ok::<(), anyhow::Error>(())
            });
            handles.push(handle);
        }

        // Wait for all to complete
        for handle in handles {
            handle.await??;
        }

        // Verify all sessions closed
        let sessions = soul.get_sessions().await;
        assert_eq!(sessions.len(), 0);

        soul.shutdown().await?;
        Ok(())
    }
}
