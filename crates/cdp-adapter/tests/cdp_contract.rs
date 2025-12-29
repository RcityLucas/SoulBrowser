//! High-level contract tests for the CDP adapter. These tests bridge the full
//! `CdpAdapter` surface to a real Chromium binary. They are ignored by default
//! because they require Chrome/Chromium on the host machine.

use std::env;
use std::sync::Arc;
use std::time::Duration;

use cdp_adapter::{event_bus, Cdp, CdpAdapter, CdpConfig};
use tokio::time::sleep;

fn contract_enabled() -> bool {
    env::var("SOULBROWSER_CDP_CONTRACT")
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

async fn setup_adapter() -> Arc<CdpAdapter> {
    let (bus, _rx) = event_bus(32);
    let adapter = Arc::new(CdpAdapter::new(CdpConfig::default(), bus));
    Arc::clone(&adapter).start().await.expect("adapter start");
    adapter
}

#[tokio::test]
#[ignore = "requires Chrome/Chromium; set SOULBROWSER_CDP_CONTRACT=1 and SOULBROWSER_USE_REAL_CHROME=1"]
async fn contract_navigate_and_type() {
    if !contract_enabled() {
        eprintln!("skipping CDP contract test (SOULBROWSER_CDP_CONTRACT not enabled)");
        return;
    }

    let adapter = setup_adapter().await;
    let page = adapter
        .create_page("about:blank")
        .await
        .expect("create initial page");

    adapter
        .navigate(page, "https://example.com", Duration::from_secs(15))
        .await
        .expect("navigate succeeds");

    adapter
        .type_text(page, "body", "soulbrowser", Duration::from_secs(5))
        .await
        .expect("type_text succeeds");

    adapter.shutdown().await;
}

#[tokio::test]
#[ignore = "requires Chrome/Chromium; set SOULBROWSER_CDP_CONTRACT=1 and SOULBROWSER_USE_REAL_CHROME=1"]
async fn contract_receives_events() {
    if !contract_enabled() {
        eprintln!("skipping CDP contract test (SOULBROWSER_CDP_CONTRACT not enabled)");
        return;
    }

    let (bus, mut rx) = event_bus(64);
    let adapter = Arc::new(CdpAdapter::new(CdpConfig::default(), bus));
    Arc::clone(&adapter).start().await.expect("adapter start");

    let _page = adapter
        .create_page("https://example.com")
        .await
        .expect("create page");

    let mut seen = 0usize;
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(10) && seen < 3 {
        if let Ok(event) = rx.try_recv() {
            println!("contract event: {}", event.method);
            seen += 1;
        } else {
            sleep(Duration::from_millis(100)).await;
        }
    }

    assert!(seen >= 1, "expected at least one CDP event");
    adapter.shutdown().await;
}
