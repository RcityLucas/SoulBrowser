//! L2 Perception Integration Tests
//!
//! Tests the multi-modal perception system (Visual + Semantic + Structural)
//! with real Chrome browser.
//!
//! Run with: SOULBROWSER_USE_REAL_CHROME=1 cargo test --test l2_perception_integration

use anyhow::{Context, Result};
use cdp_adapter::{CdpAdapter, CdpConfig};
use perceiver_hub::{PerceptionHub, PerceptionHubImpl, PerceptionOptions};
use perceiver_semantic::SemanticPerceiverImpl;
use perceiver_structural::{StructuralAdapterPort, StructuralPerceiverImpl};
use perceiver_visual::VisualPerceiverImpl;
use serde_json::json;
use soulbrowser_core_types::ExecRoute;
use soulbrowser_event_bus::EventBus;
use soulbrowser_policy_center::{InMemoryPolicyCenter, PolicyCenter};
use soulbrowser_state_center::{InMemoryStateCenter, StateCenter};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

fn is_real_chrome_enabled() -> bool {
    std::env::var("SOULBROWSER_USE_REAL_CHROME")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn default_snapshot() -> serde_json::Value {
    json!({
        "version": "1.0",
        "snapshot_name": "default",
        "rules": []
    })
}

async fn setup_browser_and_perceivers() -> Result<(
    Arc<CdpAdapter>,
    Arc<PerceptionHubImpl>,
    String,
    tokio::sync::broadcast::Receiver<cdp_adapter::RawEvent>,
)> {
    let (bus, rx) = EventBus::new(256);
    let config = CdpConfig::default();
    let adapter = Arc::new(CdpAdapter::new(config, bus));

    adapter
        .start()
        .await
        .context("failed to start CDP adapter")?;

    // Wait for initial page
    let mut event_rx = rx.resubscribe();
    let page_id = wait_for_page(&mut event_rx, Duration::from_secs(10))
        .await
        .context("waiting for initial page")?;

    // Create state and policy centers
    let state_center: Arc<InMemoryStateCenter> = Arc::new(InMemoryStateCenter::new(256));
    let state_center_dyn: Arc<dyn StateCenter> = state_center.clone();
    let policy_center: Arc<dyn PolicyCenter + Send + Sync> =
        Arc::new(InMemoryPolicyCenter::new(default_snapshot()));

    // Create structural perceiver
    let perception_port = Arc::new(StructuralAdapterPort::new(Arc::clone(&adapter)));
    let structural_perceiver = Arc::new(
        StructuralPerceiverImpl::with_state_center_and_live_policy(
            perception_port,
            state_center_dyn,
            policy_center,
        )
        .await,
    );

    // Create visual and semantic perceivers
    let visual_perceiver = Arc::new(VisualPerceiverImpl::new(Arc::clone(&adapter)));
    let semantic_perceiver = Arc::new(SemanticPerceiverImpl::new(
        structural_perceiver.clone() as Arc<dyn perceiver_structural::StructuralPerceiver>,
    ));

    // Create perception hub
    let hub = Arc::new(PerceptionHubImpl::new(
        structural_perceiver,
        visual_perceiver,
        semantic_perceiver,
    ));

    Ok((adapter, hub, page_id, event_rx))
}

async fn wait_for_page(
    rx: &mut tokio::sync::broadcast::Receiver<cdp_adapter::RawEvent>,
    timeout: Duration,
) -> Result<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Ok(cdp_adapter::RawEvent::PageLifecycle { page, phase, .. })) => {
                if phase.to_lowercase().contains("open") {
                    return Ok(page);
                }
            }
            _ => continue,
        }
    }
    anyhow::bail!("timeout waiting for page")
}

fn build_exec_route(adapter: &CdpAdapter, page_id: &str) -> Result<ExecRoute> {
    let registry = adapter.registry();
    let ctx = registry
        .get(page_id)
        .context("page not found in registry")?;

    Ok(ExecRoute {
        session: ctx.session_id.clone(),
        page: page_id.to_string(),
        frame: ctx
            .main_frame_id
            .as_ref()
            .context("no main frame")?
            .clone(),
    })
}

#[tokio::test]
async fn test_structural_perception() -> Result<()> {
    if !is_real_chrome_enabled() {
        eprintln!("Skipping real Chrome test (set SOULBROWSER_USE_REAL_CHROME=1)");
        return Ok(());
    }

    let (adapter, hub, page_id, mut _rx) = setup_browser_and_perceivers().await?;

    // Navigate to a test page
    adapter
        .navigate(page_id.clone(), "https://example.com", Duration::from_secs(10))
        .await
        .context("navigation failed")?;

    adapter
        .wait_basic(page_id.clone(), "domready".to_string(), Duration::from_secs(10))
        .await
        .context("DOM ready timeout")?;

    sleep(Duration::from_millis(500)).await;

    let exec_route = build_exec_route(&adapter, &page_id)?;

    // Perform structural-only perception
    let opts = PerceptionOptions {
        enable_structural: true,
        enable_visual: false,
        enable_semantic: false,
        enable_insights: false,
        capture_screenshot: false,
        extract_text: false,
        timeout_secs: 30,
    };

    let perception = hub.perceive(&exec_route, opts).await?;

    // Verify structural analysis
    assert!(perception.structural.dom_node_count > 0, "DOM should have nodes");
    println!("✓ Structural perception: {} DOM nodes", perception.structural.dom_node_count);

    adapter.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn test_visual_perception() -> Result<()> {
    if !is_real_chrome_enabled() {
        eprintln!("Skipping real Chrome test (set SOULBROWSER_USE_REAL_CHROME=1)");
        return Ok(());
    }

    let (adapter, hub, page_id, mut _rx) = setup_browser_and_perceivers().await?;

    adapter
        .navigate(page_id.clone(), "https://example.com", Duration::from_secs(10))
        .await?;

    adapter
        .wait_basic(page_id.clone(), "domready".to_string(), Duration::from_secs(10))
        .await?;

    sleep(Duration::from_millis(500)).await;

    let exec_route = build_exec_route(&adapter, &page_id)?;

    // Perform visual perception
    let opts = PerceptionOptions {
        enable_structural: false,
        enable_visual: true,
        enable_semantic: false,
        enable_insights: false,
        capture_screenshot: true,
        extract_text: false,
        timeout_secs: 30,
    };

    let perception = hub.perceive(&exec_route, opts).await?;

    // Verify visual analysis
    let visual = perception.visual.expect("visual analysis should be present");
    assert!(!visual.screenshot_id.is_empty(), "should have screenshot ID");
    assert!(!visual.dominant_colors.is_empty(), "should detect colors");
    assert!(visual.avg_contrast >= 0.0, "contrast should be valid");
    assert!(visual.viewport_utilization >= 0.0 && visual.viewport_utilization <= 1.0,
        "viewport utilization should be in [0,1]");

    println!("✓ Visual perception:");
    println!("  - Screenshot ID: {}", visual.screenshot_id);
    println!("  - Colors: {}", visual.dominant_colors.len());
    println!("  - Contrast: {:.2}", visual.avg_contrast);
    println!("  - Viewport: {:.1}%", visual.viewport_utilization * 100.0);

    adapter.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn test_semantic_perception() -> Result<()> {
    if !is_real_chrome_enabled() {
        eprintln!("Skipping real Chrome test (set SOULBROWSER_USE_REAL_CHROME=1)");
        return Ok(());
    }

    let (adapter, hub, page_id, mut _rx) = setup_browser_and_perceivers().await?;

    adapter
        .navigate(page_id.clone(), "https://example.com", Duration::from_secs(10))
        .await?;

    adapter
        .wait_basic(page_id.clone(), "domready".to_string(), Duration::from_secs(10))
        .await?;

    sleep(Duration::from_millis(500)).await;

    let exec_route = build_exec_route(&adapter, &page_id)?;

    // Perform semantic perception
    let opts = PerceptionOptions {
        enable_structural: false,
        enable_visual: false,
        enable_semantic: true,
        enable_insights: false,
        capture_screenshot: false,
        extract_text: true,
        timeout_secs: 30,
    };

    let perception = hub.perceive(&exec_route, opts).await?;

    // Verify semantic analysis
    let semantic = perception.semantic.expect("semantic analysis should be present");
    assert!(!semantic.language.is_empty(), "should detect language");
    assert!(semantic.language_confidence > 0.0, "should have language confidence");
    assert!(!semantic.summary.is_empty(), "should have summary");

    println!("✓ Semantic perception:");
    println!("  - Language: {} ({:.1}%)", semantic.language, semantic.language_confidence * 100.0);
    println!("  - Content: {:?}", semantic.content_type);
    println!("  - Intent: {:?}", semantic.intent);
    println!("  - Keywords: {:?}", semantic.keywords);

    adapter.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn test_multimodal_perception() -> Result<()> {
    if !is_real_chrome_enabled() {
        eprintln!("Skipping real Chrome test (set SOULBROWSER_USE_REAL_CHROME=1)");
        return Ok(());
    }

    let (adapter, hub, page_id, mut _rx) = setup_browser_and_perceivers().await?;

    adapter
        .navigate(page_id.clone(), "https://example.com", Duration::from_secs(10))
        .await?;

    adapter
        .wait_basic(page_id.clone(), "domready".to_string(), Duration::from_secs(10))
        .await?;

    sleep(Duration::from_millis(500)).await;

    let exec_route = build_exec_route(&adapter, &page_id)?;

    // Perform full multi-modal perception
    let opts = PerceptionOptions {
        enable_structural: true,
        enable_visual: true,
        enable_semantic: true,
        enable_insights: true,
        capture_screenshot: true,
        extract_text: true,
        timeout_secs: 30,
    };

    let perception = hub.perceive(&exec_route, opts).await?;

    // Verify all modalities present
    assert!(perception.structural.dom_node_count > 0, "should have structural data");
    assert!(perception.visual.is_some(), "should have visual data");
    assert!(perception.semantic.is_some(), "should have semantic data");
    assert!(perception.confidence > 0.0 && perception.confidence <= 1.0,
        "confidence should be valid");

    println!("✓ Multi-modal perception:");
    println!("  - Structural: {} nodes", perception.structural.dom_node_count);
    println!("  - Visual: present");
    println!("  - Semantic: present");
    println!("  - Insights: {}", perception.insights.len());
    println!("  - Confidence: {:.1}%", perception.confidence * 100.0);

    adapter.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn test_cross_modal_insights() -> Result<()> {
    if !is_real_chrome_enabled() {
        eprintln!("Skipping real Chrome test (set SOULBROWSER_USE_REAL_CHROME=1)");
        return Ok(());
    }

    let (adapter, hub, page_id, mut _rx) = setup_browser_and_perceivers().await?;

    // Navigate to a content-rich page that should generate insights
    adapter
        .navigate(page_id.clone(), "https://www.wikipedia.org", Duration::from_secs(15))
        .await?;

    adapter
        .wait_basic(page_id.clone(), "domready".to_string(), Duration::from_secs(15))
        .await?;

    sleep(Duration::from_secs(1)).await;

    let exec_route = build_exec_route(&adapter, &page_id)?;

    let opts = PerceptionOptions {
        enable_structural: true,
        enable_visual: true,
        enable_semantic: true,
        enable_insights: true,
        capture_screenshot: true,
        extract_text: true,
        timeout_secs: 30,
    };

    let perception = hub.perceive(&exec_route, opts).await?;

    // Should have cross-modal insights for a complex page
    println!("✓ Cross-modal insights test:");
    println!("  - Generated {} insights", perception.insights.len());

    for insight in &perception.insights {
        println!("  - [{:?}] {} (confidence: {:.0}%)",
            insight.insight_type,
            insight.description,
            insight.confidence * 100.0
        );
    }

    adapter.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn test_perception_timeout() -> Result<()> {
    if !is_real_chrome_enabled() {
        eprintln!("Skipping real Chrome test (set SOULBROWSER_USE_REAL_CHROME=1)");
        return Ok(());
    }

    let (adapter, hub, page_id, mut _rx) = setup_browser_and_perceivers().await?;

    adapter
        .navigate(page_id.clone(), "https://example.com", Duration::from_secs(10))
        .await?;

    adapter
        .wait_basic(page_id.clone(), "domready".to_string(), Duration::from_secs(10))
        .await?;

    sleep(Duration::from_millis(500)).await;

    let exec_route = build_exec_route(&adapter, &page_id)?;

    // Test with very short timeout - should still complete for simple page
    let opts = PerceptionOptions {
        enable_structural: true,
        enable_visual: true,
        enable_semantic: true,
        enable_insights: false,
        capture_screenshot: true,
        extract_text: true,
        timeout_secs: 5, // Short but reasonable timeout
    };

    let result = hub.perceive(&exec_route, opts).await;
    assert!(result.is_ok(), "should complete within timeout for simple page");

    println!("✓ Timeout handling verified");

    adapter.shutdown().await;
    Ok(())
}
