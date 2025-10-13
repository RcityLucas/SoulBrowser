use network_tap_light::{NetworkSnapshot, NetworkSummary, NetworkTapLight, PageId};

#[tokio::test]
async fn enable_and_disable_track_pages() {
    let (tap, _rx) = NetworkTapLight::new(16);
    let page = PageId::new();

    tap.enable(page).await.unwrap();
    assert!(tap.current_snapshot(page).await.is_some());

    tap.disable(page).await.unwrap();
    assert!(tap.current_snapshot(page).await.is_none());
}

#[tokio::test]
async fn snapshot_updates_are_observable() {
    let (tap, _rx) = NetworkTapLight::new(16);
    let page = PageId::new();
    tap.enable(page).await.unwrap();

    let snapshot = NetworkSnapshot {
        req: 10,
        res2xx: 9,
        res4xx: 1,
        res5xx: 0,
        inflight: 0,
        quiet: true,
        window_ms: 250,
        since_last_activity_ms: 1200,
    };

    tap.update_snapshot(page, snapshot.clone()).await.unwrap();

    let current = tap.current_snapshot(page).await.unwrap();
    assert_eq!(current.req, 10);
    assert!(current.quiet);
}

#[tokio::test]
async fn publish_summary_sends_to_subscribers() {
    let (tap, mut rx) = NetworkTapLight::new(16);
    let page = PageId::new();
    tap.enable(page).await.unwrap();

    let summary = NetworkSummary {
        page,
        window_ms: 250,
        req: 5,
        res2xx: 4,
        res4xx: 1,
        res5xx: 0,
        inflight: 0,
        quiet: false,
        since_last_activity_ms: 80,
    };

    tap.publish_summary(summary.clone());

    let received = rx.recv().await.unwrap();
    assert_eq!(received.req, 5);
    assert_eq!(received.page.0, page.0);
}
