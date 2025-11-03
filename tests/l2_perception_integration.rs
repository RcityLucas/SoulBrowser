//! L2 Perception Integration Tests
//!
//! Tests the multi-modal perception system (Visual + Semantic + Structural)
//! with real Chrome browser.
//!
//! Run with: SOULBROWSER_USE_REAL_CHROME=1 cargo test --test l2_perception_integration

use anyhow::{Context, Result};
use cdp_adapter::{
    event_bus as cdp_event_bus,
    ids::{FrameId as AdapterFrameId, PageId as AdapterPageId},
    AdapterError, Cdp, CdpAdapter, CdpConfig, RawEvent,
};
use perceiver_hub::{PerceptionHub, PerceptionHubImpl, PerceptionOptions};
use perceiver_semantic::SemanticPerceiverImpl;
use perceiver_structural::{AdapterPort, StructuralPerceiver, StructuralPerceiverImpl};
use perceiver_visual::VisualPerceiverImpl;
use soulbrowser_core_types::{
    ExecRoute, FrameId as CoreFrameId, PageId as CorePageId, SessionId as CoreSessionId,
};
use soulbrowser_policy_center::{default_snapshot, InMemoryPolicyCenter, PolicyCenter};
use soulbrowser_state_center::{InMemoryStateCenter, StateCenter};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::broadcast;
use tokio::time::sleep;

fn is_real_chrome_enabled() -> bool {
    std::env::var("SOULBROWSER_USE_REAL_CHROME")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

#[derive(Clone, Copy)]
struct PageContext {
    page: AdapterPageId,
    frame: AdapterFrameId,
}

struct PerceptionHarness {
    adapter: Arc<CdpAdapter>,
    hub: Arc<PerceptionHubImpl>,
    page_ctx: PageContext,
    _events: broadcast::Receiver<RawEvent>,
    _profile_dir: TempDir,
}

fn adapter_error(context: &str, err: AdapterError) -> anyhow::Error {
    let hint = err.hint.clone().unwrap_or_default();
    let data = err
        .data
        .as_ref()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    anyhow::anyhow!(
        "{}: kind={:?}, retriable={}, hint={}, data={}",
        context,
        err.kind,
        err.retriable,
        hint,
        data
    )
}

trait AdapterResultExt<T> {
    fn map_anyhow(self, context: &str) -> Result<T>;
}

impl<T> AdapterResultExt<T> for Result<T, AdapterError> {
    fn map_anyhow(self, context: &str) -> Result<T> {
        self.map_err(|err| adapter_error(context, err))
    }
}

async fn setup_browser_and_perceivers() -> Result<PerceptionHarness> {
    let (bus, mut rx) = cdp_event_bus(256);
    let mut config = CdpConfig::default();
    let profile_dir = tempfile::Builder::new()
        .prefix("soulbrowser-profile-")
        .tempdir()
        .context("create temp chrome profile")?;
    config.user_data_dir = profile_dir.path().to_path_buf();
    let adapter = Arc::new(CdpAdapter::new(config, bus));

    Arc::clone(&adapter)
        .start()
        .await
        .map_anyhow("start CDP adapter")?;

    // Wait for initial page and main frame
    let page_ctx = wait_for_page(&mut rx, Duration::from_secs(10)).await?;

    // Create state and policy centers
    let state_center: Arc<InMemoryStateCenter> = Arc::new(InMemoryStateCenter::new(256));
    let state_center_dyn: Arc<dyn StateCenter> = state_center.clone();
    let policy_center: Arc<dyn PolicyCenter + Send + Sync> =
        Arc::new(InMemoryPolicyCenter::new(default_snapshot()));

    // Create structural perceiver
    let perception_port = Arc::new(AdapterPort::new(Arc::clone(&adapter)));
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
        structural_perceiver.clone() as Arc<dyn StructuralPerceiver>
    ));

    // Create perception hub
    let hub = Arc::new(PerceptionHubImpl::new(
        structural_perceiver,
        visual_perceiver,
        semantic_perceiver,
    ));

    Ok(PerceptionHarness {
        adapter,
        hub,
        page_ctx,
        _events: rx,
        _profile_dir: profile_dir,
    })
}

async fn wait_for_page(
    rx: &mut broadcast::Receiver<RawEvent>,
    timeout: Duration,
) -> Result<PageContext> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Ok(RawEvent::PageLifecycle {
                page, frame, phase, ..
            })) => {
                if phase.eq_ignore_ascii_case("frame_attached") {
                    if let Some(frame_id) = frame {
                        return Ok(PageContext {
                            page,
                            frame: frame_id,
                        });
                    }
                }
            }
            _ => continue,
        }
    }
    anyhow::bail!("timeout waiting for page context")
}

fn build_exec_route(adapter: &CdpAdapter, page_ctx: &PageContext) -> Result<ExecRoute> {
    let registry = adapter.registry();
    let ctx = registry
        .get(&page_ctx.page)
        .context("page not found in registry")?;

    let session = CoreSessionId(ctx.session_id.0.to_string());
    let page = CorePageId(page_ctx.page.0.to_string());
    let frame = CoreFrameId(page_ctx.frame.0.to_string());

    Ok(ExecRoute::new(session, page, frame))
}

#[tokio::test]
#[ignore = "Requires real Chrome (set SOULBROWSER_USE_REAL_CHROME=1)"]
async fn test_structural_perception() -> Result<()> {
    if !is_real_chrome_enabled() {
        eprintln!("Skipping real Chrome test (set SOULBROWSER_USE_REAL_CHROME=1)");
        return Ok(());
    }

    let PerceptionHarness {
        adapter,
        hub,
        page_ctx,
        ..
    } = setup_browser_and_perceivers().await?;

    // Navigate to a test page
    adapter
        .navigate(
            page_ctx.page,
            "https://example.com",
            Duration::from_secs(10),
        )
        .await
        .map_anyhow("navigation failed")?;

    adapter
        .wait_basic(
            page_ctx.page,
            "domready".to_string(),
            Duration::from_secs(10),
        )
        .await
        .map_anyhow("DOM ready timeout")?;

    sleep(Duration::from_millis(500)).await;

    let exec_route = build_exec_route(&adapter, &page_ctx)?;

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
    assert!(
        perception.structural.dom_node_count > 0,
        "DOM should have nodes"
    );
    println!(
        "✓ Structural perception: {} DOM nodes",
        perception.structural.dom_node_count
    );

    adapter.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "Requires real Chrome (set SOULBROWSER_USE_REAL_CHROME=1)"]
async fn test_visual_perception() -> Result<()> {
    if !is_real_chrome_enabled() {
        eprintln!("Skipping real Chrome test (set SOULBROWSER_USE_REAL_CHROME=1)");
        return Ok(());
    }

    let PerceptionHarness {
        adapter,
        hub,
        page_ctx,
        ..
    } = setup_browser_and_perceivers().await?;

    adapter
        .navigate(
            page_ctx.page,
            "https://example.com",
            Duration::from_secs(10),
        )
        .await
        .map_anyhow("navigation failed")?;

    adapter
        .wait_basic(
            page_ctx.page,
            "domready".to_string(),
            Duration::from_secs(10),
        )
        .await
        .map_anyhow("DOM ready timeout")?;

    sleep(Duration::from_millis(500)).await;

    let exec_route = build_exec_route(&adapter, &page_ctx)?;

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
    let visual = perception
        .visual
        .expect("visual analysis should be present");
    assert!(
        !visual.screenshot_id.is_empty(),
        "should have screenshot ID"
    );
    assert!(!visual.dominant_colors.is_empty(), "should detect colors");
    assert!(visual.avg_contrast >= 0.0, "contrast should be valid");
    assert!(
        visual.viewport_utilization >= 0.0 && visual.viewport_utilization <= 1.0,
        "viewport utilization should be in [0,1]"
    );

    println!("✓ Visual perception:");
    println!("  - Screenshot ID: {}", visual.screenshot_id);
    println!("  - Colors: {}", visual.dominant_colors.len());
    println!("  - Contrast: {:.2}", visual.avg_contrast);
    println!("  - Viewport: {:.1}%", visual.viewport_utilization * 100.0);

    adapter.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "Requires real Chrome (set SOULBROWSER_USE_REAL_CHROME=1)"]
async fn test_semantic_perception() -> Result<()> {
    if !is_real_chrome_enabled() {
        eprintln!("Skipping real Chrome test (set SOULBROWSER_USE_REAL_CHROME=1)");
        return Ok(());
    }

    let PerceptionHarness {
        adapter,
        hub,
        page_ctx,
        ..
    } = setup_browser_and_perceivers().await?;

    adapter
        .navigate(
            page_ctx.page,
            "https://example.com",
            Duration::from_secs(10),
        )
        .await
        .map_anyhow("navigation failed")?;

    adapter
        .wait_basic(
            page_ctx.page,
            "domready".to_string(),
            Duration::from_secs(10),
        )
        .await
        .map_anyhow("DOM ready timeout")?;

    sleep(Duration::from_millis(500)).await;

    let exec_route = build_exec_route(&adapter, &page_ctx)?;

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
    let semantic = perception
        .semantic
        .expect("semantic analysis should be present");
    assert!(!semantic.language.is_empty(), "should detect language");
    assert!(
        semantic.language_confidence > 0.0,
        "should have language confidence"
    );
    assert!(!semantic.summary.is_empty(), "should have summary");

    println!("✓ Semantic perception:");
    println!(
        "  - Language: {} ({:.1}%)",
        semantic.language,
        semantic.language_confidence * 100.0
    );
    println!("  - Content: {:?}", semantic.content_type);
    println!("  - Intent: {:?}", semantic.intent);
    println!("  - Keywords: {:?}", semantic.keywords);

    adapter.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "Requires real Chrome (set SOULBROWSER_USE_REAL_CHROME=1)"]
async fn test_multimodal_perception() -> Result<()> {
    if !is_real_chrome_enabled() {
        eprintln!("Skipping real Chrome test (set SOULBROWSER_USE_REAL_CHROME=1)");
        return Ok(());
    }

    let PerceptionHarness {
        adapter,
        hub,
        page_ctx,
        ..
    } = setup_browser_and_perceivers().await?;

    adapter
        .navigate(
            page_ctx.page,
            "https://example.com",
            Duration::from_secs(10),
        )
        .await
        .map_anyhow("navigation failed")?;

    adapter
        .wait_basic(
            page_ctx.page,
            "domready".to_string(),
            Duration::from_secs(10),
        )
        .await
        .map_anyhow("DOM ready timeout")?;

    sleep(Duration::from_millis(500)).await;

    let exec_route = build_exec_route(&adapter, &page_ctx)?;

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
    assert!(
        perception.structural.dom_node_count > 0,
        "should have structural data"
    );
    assert!(perception.visual.is_some(), "should have visual data");
    assert!(perception.semantic.is_some(), "should have semantic data");
    assert!(
        perception.confidence > 0.0 && perception.confidence <= 1.0,
        "confidence should be valid"
    );

    println!("✓ Multi-modal perception:");
    println!(
        "  - Structural: {} nodes",
        perception.structural.dom_node_count
    );
    println!("  - Visual: present");
    println!("  - Semantic: present");
    println!("  - Insights: {}", perception.insights.len());
    println!("  - Confidence: {:.1}%", perception.confidence * 100.0);

    adapter.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "Requires real Chrome (set SOULBROWSER_USE_REAL_CHROME=1)"]
async fn test_cross_modal_insights() -> Result<()> {
    if !is_real_chrome_enabled() {
        eprintln!("Skipping real Chrome test (set SOULBROWSER_USE_REAL_CHROME=1)");
        return Ok(());
    }

    let PerceptionHarness {
        adapter,
        hub,
        page_ctx,
        ..
    } = setup_browser_and_perceivers().await?;

    // Navigate to a content-rich page that should generate insights
    adapter
        .navigate(
            page_ctx.page,
            "https://www.wikipedia.org",
            Duration::from_secs(15),
        )
        .await
        .map_anyhow("navigation failed")?;

    adapter
        .wait_basic(
            page_ctx.page,
            "domready".to_string(),
            Duration::from_secs(15),
        )
        .await
        .map_anyhow("DOM ready timeout")?;

    sleep(Duration::from_secs(1)).await;

    let exec_route = build_exec_route(&adapter, &page_ctx)?;

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
        println!(
            "  - [{:?}] {} (confidence: {:.0}%)",
            insight.insight_type,
            insight.description,
            insight.confidence * 100.0
        );
    }

    adapter.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "Requires real Chrome (set SOULBROWSER_USE_REAL_CHROME=1)"]
async fn test_perception_timeout() -> Result<()> {
    if !is_real_chrome_enabled() {
        eprintln!("Skipping real Chrome test (set SOULBROWSER_USE_REAL_CHROME=1)");
        return Ok(());
    }

    let PerceptionHarness {
        adapter,
        hub,
        page_ctx,
        ..
    } = setup_browser_and_perceivers().await?;

    adapter
        .navigate(
            page_ctx.page,
            "https://example.com",
            Duration::from_secs(10),
        )
        .await
        .map_anyhow("navigation failed")?;

    adapter
        .wait_basic(
            page_ctx.page,
            "domready".to_string(),
            Duration::from_secs(10),
        )
        .await
        .map_anyhow("DOM ready timeout")?;

    sleep(Duration::from_millis(500)).await;

    let exec_route = build_exec_route(&adapter, &page_ctx)?;

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
    assert!(
        result.is_ok(),
        "should complete within timeout for simple page"
    );

    println!("✓ Timeout handling verified");

    adapter.shutdown().await;
    Ok(())
}
