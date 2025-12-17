#[cfg(feature = "full-stack")]
mod full_stack {

    //! Stress tests for SoulBrowser

    use anyhow::Result;
    use chrono::Utc;
    use l4_event_store::{Event, EventData, EventStore, EventStoreConfig, EventType};
    use l6_metrics::{MetricsCenter, MetricsConfig};
    use l6_timeline::{
        EntryData as TimelineData, EntryType, Severity, TimelineConfig, TimelineEntry,
        TimelineManager,
    };
    use soulbrowser::Soul;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::sync::Semaphore;
    use uuid::Uuid;

    /// Stress test concurrent session creation
    #[tokio::test]
    #[ignore] // Run with: cargo test stress_test_concurrent_sessions -- --ignored
    async fn stress_test_concurrent_sessions() -> Result<()> {
        let soul = Soul::new().await?;
        let soul = Arc::new(soul);
        let semaphore = Arc::new(Semaphore::new(10)); // Limit concurrent sessions

        let mut handles = Vec::new();

        // Create 50 sessions concurrently
        for i in 0..50 {
            let soul = soul.clone();
            let sem = semaphore.clone();

            let handle = tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();

                let session = soul.create_session().await?;
                println!("Created session {}: {}", i, session.id());

                // Simulate some work
                tokio::time::sleep(Duration::from_millis(100)).await;

                soul.close_session(session.id()).await?;
                println!("Closed session {}", i);

                Ok::<(), anyhow::Error>(())
            });

            handles.push(handle);
        }

        // Wait for all sessions to complete
        for handle in handles {
            handle.await??;
        }

        // Verify all sessions are closed
        let remaining = soul.get_sessions().await;
        assert_eq!(remaining.len(), 0, "All sessions should be closed");

        soul.shutdown().await?;
        Ok(())
    }

    /// Stress test event store with high throughput
    #[tokio::test]
    #[ignore]
    async fn stress_test_event_store_throughput() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let config = EventStoreConfig {
            storage_dir: temp_dir.path().to_path_buf(),
            max_events_per_session: 100000,
            ..Default::default()
        };

        let store = Arc::new(EventStore::new(config).await?);
        let mut handles = Vec::new();

        // Generate events from multiple threads
        for thread_id in 0..10 {
            let store = store.clone();

            let handle = tokio::spawn(async move {
                for i in 0..1000 {
                    let event = Event {
                        id: Uuid::new_v4().to_string(),
                        session_id: format!("stress-session-{}", thread_id),
                        timestamp: Utc::now(),
                        event_type: EventType::Action,
                        data: EventData::Action {
                            action_type: "test".to_string(),
                            target: Some(format!("element-{}", i)),
                            parameters: serde_json::json!({"thread": thread_id, "index": i}),
                        },
                        sequence: i as u64,
                        parent_id: None,
                        tags: vec!["stress-test".to_string()],
                    };

                    store.store(event).await?;

                    if i % 100 == 0 {
                        println!("Thread {} stored {} events", thread_id, i);
                    }
                }
                Ok::<(), anyhow::Error>(())
            });

            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.await??;
        }

        // Query to verify
        let query = l4_event_store::QueryBuilder::new()
            .tag("stress-test")
            .build();

        let events = store.query(query).await?;
        println!("Total events stored: {}", events.len());
        assert!(
            events.len() >= 9000,
            "Should have stored at least 9000 events"
        );

        Ok(())
    }

    /// Stress test metrics collection
    #[tokio::test]
    #[ignore]
    async fn stress_test_metrics_collection() -> Result<()> {
        let config = MetricsConfig {
            collection_interval: Duration::from_millis(10),
            ..Default::default()
        };

        let metrics = Arc::new(MetricsCenter::new(config).await?);
        metrics.start().await;

        let mut handles = Vec::new();

        // Generate metrics from multiple sources
        for source_id in 0..20 {
            let metrics = metrics.clone();

            let handle = tokio::spawn(async move {
                for i in 0..500 {
                    metrics.inc_counter(&format!("counter_{}", source_id), 1.0);
                    metrics.set_gauge(&format!("gauge_{}", source_id), i as f64);
                    metrics
                        .observe_histogram(&format!("histogram_{}", source_id), (i as f64) * 0.01);

                    if i % 100 == 0 {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
            });

            handles.push(handle);
        }

        // Wait for metrics generation
        for handle in handles {
            handle.await?;
        }

        // Give time for final collection
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Check metrics
        let all_metrics = metrics.get_all();
        println!("Total metric keys: {}", all_metrics.len());
        assert!(
            all_metrics.len() >= 30,
            "Should have at least 30 different metrics"
        );

        Ok(())
    }

    /// Stress test timeline with rapid entries
    #[tokio::test]
    #[ignore]
    async fn stress_test_timeline_entries() -> Result<()> {
        let config = TimelineConfig {
            max_events: 50000,
            ..Default::default()
        };

        let manager = Arc::new(TimelineManager::new(config).await?);
        let session_id = "stress-timeline";

        manager.create_timeline(session_id).await?;

        let mut handles = Vec::new();

        // Add entries from multiple sources
        for source_id in 0..10 {
            let manager = manager.clone();

            let handle = tokio::spawn(async move {
                for i in 0..1000 {
                    let entry = TimelineEntry {
                        id: Uuid::new_v4().to_string(),
                        timestamp: Utc::now(),
                        entry_type: match i % 4 {
                            0 => EntryType::Navigation,
                            1 => EntryType::Interaction,
                            2 => EntryType::Network,
                            _ => EntryType::Performance,
                        },
                        duration: Some(chrono::Duration::milliseconds(i as i64)),
                        data: TimelineData::Custom(serde_json::json!({
                            "source": source_id,
                            "index": i,
                        })),
                        tags: vec!["stress".to_string()],
                        severity: Severity::Info,
                    };

                    manager.add_entry(session_id, entry).await?;

                    if i % 200 == 0 {
                        println!("Source {} added {} entries", source_id, i);
                    }
                }
                Ok::<(), anyhow::Error>(())
            });

            handles.push(handle);
        }

        // Add milestones concurrently
        for i in 0..10 {
            manager
                .add_milestone(
                    session_id,
                    format!("Milestone {}", i),
                    Some(format!("Stress test milestone {}", i)),
                )
                .await?;
        }

        // Wait for all entries
        for handle in handles {
            handle.await??;
        }

        // Analyze the timeline
        let analysis = manager.analyze_timeline(session_id).await?;
        println!(
            "Timeline analysis: {} total entries",
            analysis.summary.total_entries
        );
        assert!(
            analysis.summary.total_entries >= 9000,
            "Should have at least 9000 entries"
        );

        // Test visualization under load
        let _html = manager
            .visualize_timeline(session_id, l6_timeline::VisualizationFormat::Html)
            .await?;
        let _text = manager
            .visualize_timeline(session_id, l6_timeline::VisualizationFormat::Text)
            .await?;

        manager.complete_timeline(session_id).await?;

        Ok(())
    }

    /// Stress test flow execution with complex flows
    #[tokio::test]
    #[ignore]
    async fn stress_test_flow_execution() -> Result<()> {
        use l3_flow::{FlowBuilder, FlowExecutor};
        use l3_primitives::{ExecutionContext, PrimitiveAction};

        let soul = Soul::new().await?;
        let session = soul.create_session().await?;

        // Build a complex flow with many steps
        let mut builder = FlowBuilder::new("Stress Flow");

        for i in 0..100 {
            builder = builder.action(
                format!("step_{}", i),
                format!("Step {}", i),
                PrimitiveAction::Execute {
                    script: format!("console.log('Step {}')", i),
                },
            );
        }

        let flow = builder.build();
        flow.validate()?;

        let executor = FlowExecutor::new();
        let context = ExecutionContext {
            cdp_session: session.cdp_session.clone(),
            state_center: session.state_center.clone(),
            policy_engine: Arc::new(l1_policy::PolicyEngine::new(Default::default())),
            retry_policy: l3_primitives::RetryPolicy::default(),
            timeout_policy: l3_primitives::TimeoutPolicy::relaxed(),
            recovery_strategy: l3_primitives::RecoveryStrategy::default(),
        };

        // Execute multiple flows concurrently
        let mut handles = Vec::new();

        for i in 0..5 {
            let flow = flow.clone();
            let executor = executor.clone();
            let context = context.clone();

            let handle = tokio::spawn(async move {
                println!("Starting flow execution {}", i);
                let result = executor.execute(flow, &context).await?;
                println!("Completed flow execution {}: success={}", i, result.success);
                Ok::<(), anyhow::Error>(())
            });

            handles.push(handle);
        }

        for handle in handles {
            handle.await??;
        }

        soul.shutdown().await?;
        Ok(())
    }

    /// Stress test memory usage with snapshot store
    #[tokio::test]
    #[ignore]
    async fn stress_test_snapshot_memory() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let soul = Soul::new().await?;
        let session = soul.create_session().await?;
        let session_id = session.id().to_string();

        let config = l4_snapshot_store::SnapshotConfig {
            storage_dir: temp_dir.path().to_path_buf(),
            max_snapshots: 1000,
            include_screenshots: false, // Disable to reduce memory
            include_dom: true,
            compression: true,
            ..Default::default()
        };

        let store = l4_snapshot_store::SnapshotStore::new(config).await?;

        // Capture many snapshots
        for i in 0..100 {
            let options = l4_snapshot_store::CaptureOptions {
                include_screenshot: false,
                include_dom: true,
                include_styles: i % 10 == 0, // Every 10th includes styles
                include_js_state: i % 20 == 0, // Every 20th includes JS state
                ..Default::default()
            };

            let snapshot = store
                .capture(&session_id, &session.cdp_session, options)
                .await?;

            if i % 10 == 0 {
                println!("Captured {} snapshots, latest: {}", i, snapshot.id);
            }

            // Small delay to simulate real usage
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Query all snapshots
        let query = l4_snapshot_store::SnapshotQuery::builder()
            .session_id(&session_id)
            .build();

        let snapshots = store.query(query).await?;
        println!("Total snapshots stored: {}", snapshots.len());
        assert!(snapshots.len() >= 100, "Should have at least 100 snapshots");

        // Test diff on random pairs
        if snapshots.len() >= 2 {
            for _ in 0..10 {
                let idx1 = rand::random::<usize>() % snapshots.len();
                let idx2 = rand::random::<usize>() % snapshots.len();

                if idx1 != idx2 {
                    let _diff = store.diff(&snapshots[idx1].id, &snapshots[idx2].id).await?;
                }
            }
        }

        soul.shutdown().await?;
        Ok(())
    }

    /// Helper function to monitor resource usage
    async fn monitor_resources(name: &str, duration: Duration) {
        let start = tokio::time::Instant::now();

        while start.elapsed() < duration {
            // In a real implementation, would use sysinfo crate
            println!("[{}] Monitoring resources...", name);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
}
