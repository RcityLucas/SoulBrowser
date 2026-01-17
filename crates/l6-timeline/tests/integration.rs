use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use l6_timeline::adapters::TimelineRuntimeEvent;
use l6_timeline::api::{Timeline, TimelineService};
use l6_timeline::model::{By, ExportReq, View};
use l6_timeline::policy::{TimelinePolicyHandle, TimelinePolicyView};
use l6_timeline::TlError;
use soulbrowser_core_types::{ActionId, ExecRoute, FrameId, PageId, SessionId};
use soulbrowser_event_bus::{to_mpsc, InMemoryBus};
use soulbrowser_event_store::api::InMemoryEventStore;
use soulbrowser_event_store::model::{
    AppendMeta, EventEnvelope as EsEnvelope, EventScope, EventSource, LogLevel,
};
use soulbrowser_event_store::EsPolicyView;
use soulbrowser_event_store::EventStore;
use soulbrowser_state_center::{DispatchEvent, InMemoryStateCenter, StateCenter, StateEvent};
use std::sync::Arc;
use tokio::time::{timeout, Duration};

fn make_event(action: &str, kind: &str, ts_mono: u128, payload: serde_json::Value) -> EsEnvelope {
    let session = SessionId("session-1".into());
    let page = PageId("page-1".into());
    let frame = FrameId("frame-1".into());
    EsEnvelope {
        event_id: format!("{}-{}", kind, ts_mono),
        ts_mono,
        ts_wall: Utc.timestamp_millis_opt(ts_mono as i64).unwrap(),
        scope: EventScope {
            session: Some(session),
            page: Some(page),
            frame: Some(frame),
            action: Some(ActionId(action.into())),
            ..Default::default()
        },
        source: EventSource::L5,
        kind: kind.into(),
        level: LogLevel::Info,
        payload,
        artifacts: Vec::new(),
        tags: Vec::new(),
    }
}

fn make_dispatch_success(action: &ActionId) -> StateEvent {
    let route = ExecRoute::new(
        SessionId("session-1".into()),
        PageId("page-1".into()),
        FrameId("frame-1".into()),
    );
    StateEvent::dispatch_success(DispatchEvent::success(
        action.clone(),
        Some("task-1".into()),
        route,
        "click".into(),
        "mutex".into(),
        1,
        10,
        20,
        0,
        4,
        None,
    ))
}

fn service_with_policy(
    event_store: Arc<InMemoryEventStore>,
    state_center: Option<Arc<InMemoryStateCenter>>,
    events_bus: Option<
        Arc<dyn soulbrowser_event_bus::EventBus<TimelineRuntimeEvent> + Send + Sync>,
    >,
    policy: &TimelinePolicyView,
) -> (TimelineService, TimelinePolicyHandle) {
    TimelineService::with_runtime_and_policy(
        event_store,
        state_center,
        events_bus,
        TimelinePolicyHandle::new_with(policy.clone()),
    )
}

#[tokio::test]
async fn export_records_pipeline_produces_jsonl() {
    let es_policy = EsPolicyView::default();
    let event_store = InMemoryEventStore::new(es_policy);
    let action_id = ActionId("act-1".into());

    event_store
        .append_event(
            make_event(
                &action_id.0,
                "ACT_STARTED",
                1_000,
                serde_json::json!({"tool":"click","primitive":"click"}),
            ),
            AppendMeta::default(),
        )
        .await
        .unwrap();
    event_store
        .append_event(
            make_event(
                &action_id.0,
                "VIS_CAPTURED",
                1_100,
                serde_json::json!({"pix_ids":["pix-1"],"struct_ids":["struct-1"]}),
            ),
            AppendMeta::default(),
        )
        .await
        .unwrap();
    event_store
        .append_event(
            make_event(
                &action_id.0,
                "GATE_DECISION",
                1_200,
                serde_json::json!({"pass":true}),
            ),
            AppendMeta::default(),
        )
        .await
        .unwrap();

    let state_center = Arc::new(soulbrowser_state_center::InMemoryStateCenter::new(128));
    state_center
        .append(make_dispatch_success(&action_id))
        .await
        .unwrap();

    let bus_arc = InMemoryBus::<TimelineRuntimeEvent>::new(16);
    let mut rx = to_mpsc(Arc::clone(&bus_arc), 16);
    let bus_trait: Arc<dyn soulbrowser_event_bus::EventBus<TimelineRuntimeEvent> + Send + Sync> =
        bus_arc.clone();

    let mut policy = TimelinePolicyView::default();
    policy.log_enable = false;
    let (service, _policy_handle) = service_with_policy(
        Arc::clone(&event_store),
        Some(Arc::clone(&state_center)),
        Some(bus_trait.clone()),
        &policy,
    );

    let req = ExportReq {
        view: View::Records,
        by: By::Action {
            action_id: action_id.0.clone(),
        },
        policy_overrides: None,
    };

    let result = service.export(req).await.expect("export succeeds");
    let lines = result.lines.expect("lines");
    assert!(lines
        .iter()
        .any(|line| line.contains("\"type\":\"header\"")));
    assert!(lines
        .iter()
        .any(|line| line.contains("\"type\":\"footer\"")));

    let started = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("event bus timeout")
        .expect("event bus closed");
    matches!(started, TimelineRuntimeEvent::ExportStarted { .. });
}

#[tokio::test]
async fn build_replay_returns_minimal_bundle() {
    let es_policy = EsPolicyView::default();
    let event_store = InMemoryEventStore::new(es_policy);
    let action_id = ActionId("act-1".into());

    event_store
        .append_event(
            make_event(
                &action_id.0,
                "ACT_STARTED",
                1_000,
                serde_json::json!({"tool":"click"}),
            ),
            AppendMeta::default(),
        )
        .await
        .unwrap();

    event_store
        .append_event(
            make_event(
                &action_id.0,
                "ACT_FINISHED",
                1_200,
                serde_json::json!({"ok":true}),
            ),
            AppendMeta::default(),
        )
        .await
        .unwrap();

    let mut policy = TimelinePolicyView::default();
    policy.log_enable = false;
    let (service, _policy_handle) =
        service_with_policy(Arc::clone(&event_store), None, None, &policy);

    let bundle = service
        .build_replay(&action_id.0)
        .await
        .expect("replay available");
    assert_eq!(bundle.action_id, action_id.0);
    assert_eq!(bundle.timeline.len(), 2);
}

#[tokio::test]
async fn policy_hot_update_is_reflected() {
    let es_policy = EsPolicyView::default();
    let event_store = InMemoryEventStore::new(es_policy);
    let action_id = ActionId("act-1".into());

    event_store
        .append_event(
            make_event(
                &action_id.0,
                "ACT_STARTED",
                1_000,
                serde_json::json!({"tool":"click"}),
            ),
            AppendMeta::default(),
        )
        .await
        .unwrap();
    event_store
        .append_event(
            make_event(
                &action_id.0,
                "ACT_FINISHED",
                1_200,
                serde_json::json!({"ok":true}),
            ),
            AppendMeta::default(),
        )
        .await
        .unwrap();

    let mut policy = TimelinePolicyView::default();
    policy.log_enable = false;
    let (service, policy_handle) =
        service_with_policy(Arc::clone(&event_store), None, None, &policy);

    policy.max_lines = 1;
    policy_handle.update(policy.clone());

    let req = ExportReq {
        view: View::Records,
        by: By::Action {
            action_id: action_id.0.clone(),
        },
        policy_overrides: None,
    };
    let result = service.export(req).await.expect("export succeeds");
    assert!(result.stats.truncated);
    assert_eq!(result.stats.total_lines, 1);
}

#[tokio::test]
async fn export_timeline_view_includes_frames() {
    let es_policy = EsPolicyView::default();
    let event_store = InMemoryEventStore::new(es_policy);
    let action_id = ActionId("act-1".into());

    event_store
        .append_event(
            make_event(
                &action_id.0,
                "ACT_STARTED",
                1_000,
                serde_json::json!({"tool":"click"}),
            ),
            AppendMeta::default(),
        )
        .await
        .unwrap();
    event_store
        .append_event(
            make_event(
                &action_id.0,
                "GATE_DECISION",
                1_300,
                serde_json::json!({"pass":true}),
            ),
            AppendMeta::default(),
        )
        .await
        .unwrap();

    let mut policy = TimelinePolicyView::default();
    policy.log_enable = false;
    let (service, _policy_handle) =
        service_with_policy(Arc::clone(&event_store), None, None, &policy);
    let req = ExportReq {
        view: View::Timeline,
        by: By::Action {
            action_id: action_id.0.clone(),
        },
        policy_overrides: None,
    };
    let result = service.export(req).await.expect("export succeeds");
    let lines = result.lines.expect("lines");
    assert!(lines
        .iter()
        .any(|line| line.contains("\"type\":\"timeline\"")));
}

#[tokio::test]
async fn flow_selector_uses_session_scope() {
    let es_policy = EsPolicyView::default();
    let event_store = InMemoryEventStore::new(es_policy);
    let action_id = ActionId("act-1".into());

    event_store
        .append_event(
            make_event(
                &action_id.0,
                "ACT_STARTED",
                1_000,
                serde_json::json!({"tool":"click"}),
            ),
            AppendMeta::default(),
        )
        .await
        .unwrap();

    let mut policy = TimelinePolicyView::default();
    policy.log_enable = false;
    let (service, _policy_handle) =
        service_with_policy(Arc::clone(&event_store), None, None, &policy);
    let req = ExportReq {
        view: View::Records,
        by: By::Flow {
            flow_id: "session-1".into(),
        },
        policy_overrides: None,
    };
    let result = service.export(req).await.expect("export succeeds");
    assert!(result
        .lines
        .unwrap()
        .iter()
        .any(|line| line.contains("ACT_STARTED")));
}

#[tokio::test]
async fn range_selector_rejects_when_exceeding_policy() {
    let es_policy = EsPolicyView::default();
    let event_store = InMemoryEventStore::new(es_policy);

    let mut policy = TimelinePolicyView::default();
    policy.log_enable = false;
    policy.max_time_range_ms = 1_000;
    let (service, _policy_handle) =
        service_with_policy(Arc::clone(&event_store), None, None, &policy);
    let since = Utc::now() - ChronoDuration::minutes(5);
    let until = Utc::now();

    let req = ExportReq {
        view: View::Records,
        by: By::Range { since, until },
        policy_overrides: None,
    };
    let err = service.export(req).await.expect_err("range too large");
    assert!(matches!(err, TlError::RangeTooLarge));
}

#[tokio::test]
async fn build_replay_handles_missing_action_gracefully() {
    let es_policy = EsPolicyView::default();
    let event_store = InMemoryEventStore::new(es_policy);
    let mut policy = TimelinePolicyView::default();
    policy.log_enable = false;
    let (service, _policy_handle) =
        service_with_policy(Arc::clone(&event_store), None, None, &policy);
    let bundle = service
        .build_replay("missing")
        .await
        .expect("missing actions return empty bundle");
    assert!(bundle.action_id.is_empty());
    assert!(bundle.timeline.is_empty());
}
