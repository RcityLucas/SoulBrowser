use std::time::Duration;

use anyhow::Result;
use serde_json::json;
use soulbrowser_cli::app_context::AppContext;
use soulbrowser_cli::l0_bridge::L0Handles;
use soulbrowser_core_types::{RoutingHint, TaskId, ToolCall};
use soulbrowser_policy_center::RuntimeOverrideSpec;
use soulbrowser_scheduler::metrics;
use soulbrowser_scheduler::model::{CallOptions, DispatchRequest, Priority, RetryOpt};
use soulbrowser_scheduler::Dispatcher;
use tokio::time::sleep;

fn build_dispatch_request(call_id: &str, url: &str) -> DispatchRequest {
    DispatchRequest {
        tool_call: ToolCall {
            tool: "browser.navigate".to_string(),
            call_id: Some(call_id.to_string()),
            task_id: Some(TaskId::new()),
            payload: json!({ "url": url }),
        },
        options: CallOptions {
            timeout: Duration::from_millis(50),
            priority: Priority::Standard,
            interruptible: true,
            retry: RetryOpt {
                max: 0,
                backoff: Duration::from_millis(10),
            },
        },
        routing_hint: Some(RoutingHint::default()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l1_end_to_end_flow() -> Result<()> {
    let context = AppContext::new("test-tenant".into(), None, &[]).await?;

    // Policy override propagates to scheduler
    let initial_policy = context.policy_center().snapshot().await;
    let override_spec = RuntimeOverrideSpec {
        path: "scheduler.limits.global_slots".into(),
        value: json!(2),
        owner: "tests".into(),
        reason: "e2e override".into(),
        ttl_seconds: 0,
    };
    context
        .policy_center()
        .apply_override(override_spec)
        .await?;
    sleep(Duration::from_millis(50)).await;
    let updated_policy = context.policy_center().snapshot().await;
    assert_eq!(updated_policy.scheduler.limits.global_slots, 2);
    assert!(updated_policy.rev > initial_policy.rev);

    // Dispatch a tool request and wait for completion
    let initial_stats = context.state_center_stats();
    let scheduler = context.scheduler_service();
    let submit = scheduler.submit(build_dispatch_request("call-1", "https://example.com"));
    let handle = submit.await?;
    let _ = handle.receiver.await.expect("dispatch result");
    sleep(Duration::from_millis(50)).await;
    let post_dispatch_stats = context.state_center_stats();
    assert!(post_dispatch_stats.total_events >= initial_stats.total_events + 1);

    // Cancel path using call_id while global slots are throttled
    let cancel_policy = RuntimeOverrideSpec {
        path: "scheduler.limits.global_slots".into(),
        value: json!(0),
        owner: "tests".into(),
        reason: "throttle for cancel".into(),
        ttl_seconds: 0,
    };
    context
        .policy_center()
        .apply_override(cancel_policy)
        .await?;
    sleep(Duration::from_millis(30)).await;

    let cancel_submit = scheduler.submit(build_dispatch_request(
        "cancel-me",
        "https://cancel.example",
    ));
    let cancel_handle = cancel_submit.await?;
    let cancelled = scheduler.cancel_call("cancel-me").await?;
    assert!(cancelled);
    drop(cancel_handle);

    // Simulate L0 lifecycle + health events via bridge
    let handles: &L0Handles = context.l0_handles();
    let cdp_page = cdp_adapter::ids::PageId::new();
    handles
        .cdp_sender
        .send(cdp_adapter::events::RawEvent::PageLifecycle {
            page: cdp_page,
            frame: None,
            phase: "opened".into(),
            ts: 0,
        })
        .expect("send lifecycle");
    sleep(Duration::from_millis(50)).await;

    handles
        .cdp_sender
        .send(cdp_adapter::events::RawEvent::NetworkSummary {
            page: cdp_page,
            req: 5,
            res2xx: 4,
            res4xx: 1,
            res5xx: 0,
            inflight: 1,
            quiet: false,
            window_ms: 1_000,
            since_last_activity_ms: 0,
        })
        .expect("send network summary");
    sleep(Duration::from_millis(50)).await;

    let registry = context.registry();
    let matching_health = registry
        .pages
        .iter()
        .find_map(|entry| {
            let ctx = entry.value().read();
            if ctx.health.request_count == 5 {
                Some(ctx.health.clone())
            } else {
                None
            }
        })
        .expect("page health updated");
    assert_eq!(matching_health.request_count, 5);
    assert!(!matching_health.quiet);

    // Scheduler metrics reflect recorded activity
    let scheduler_metrics = metrics::snapshot();
    assert!(scheduler_metrics.enqueued >= 2);

    Ok(())
}
