use crate::adapters::{
    BusEventsPort, EventStoreAdapter, NoopEventsPort, StateCenterAdapter, TimelineRuntimeEvent,
};
use crate::errors::{TlError, TlResult};
use crate::export::jsonl::{serialize_lines, write_lines};
use crate::model::{ExportReq, ExportResult, JsonlLine, QueryDigest, ReplayBundle};
use crate::policy::{TimelinePolicyHandle, TimelinePolicyView};
use crate::ports::{EventStorePort, EventsPort, PolicyPort, StateCenterPort};
use crate::reader::{build_plan, describe_source, merge_outcome, run_fetch};
use crate::stitch::build::{build_footer, build_header, build_lines, BuildOutput};
use async_trait::async_trait;
use serde_json::to_value;
use soulbrowser_event_bus::EventBus;
use soulbrowser_event_store::EventStore as RuntimeEventStore;
use soulbrowser_state_center::InMemoryStateCenter;
use std::sync::Arc;
use std::time::Instant;

#[async_trait]
pub trait Timeline: Send + Sync {
    async fn export(&self, req: ExportReq) -> TlResult<ExportResult>;
    async fn build_replay(&self, action_id: &str) -> TlResult<ReplayBundle>;
    fn policy_view(&self) -> TimelinePolicyView;
}

pub struct TimelineService {
    event_store: Arc<dyn EventStorePort>,
    state_center: Option<Arc<dyn StateCenterPort>>,
    policy: Arc<dyn PolicyPort>,
    events: Arc<dyn EventsPort>,
}

impl TimelineService {
    pub fn new(
        event_store: Arc<dyn EventStorePort>,
        state_center: Option<Arc<dyn StateCenterPort>>,
        policy: Arc<dyn PolicyPort>,
        events: Arc<dyn EventsPort>,
    ) -> Self {
        Self {
            event_store,
            state_center,
            policy,
            events,
        }
    }

    pub fn with_runtime(
        event_store: Arc<dyn RuntimeEventStore>,
        state_center: Option<Arc<InMemoryStateCenter>>,
        events_bus: Option<Arc<dyn EventBus<TimelineRuntimeEvent> + Send + Sync>>,
    ) -> Self {
        let (service, _) = Self::with_runtime_and_policy(
            event_store,
            state_center,
            events_bus,
            TimelinePolicyHandle::global(),
        );
        service
    }

    pub fn with_runtime_and_policy(
        event_store: Arc<dyn RuntimeEventStore>,
        state_center: Option<Arc<InMemoryStateCenter>>,
        events_bus: Option<Arc<dyn EventBus<TimelineRuntimeEvent> + Send + Sync>>,
        policy_handle: TimelinePolicyHandle,
    ) -> (Self, TimelinePolicyHandle) {
        let event_store_port: Arc<dyn EventStorePort> =
            Arc::new(EventStoreAdapter::new(event_store));
        let state_center_port: Option<Arc<dyn StateCenterPort>> = state_center
            .map(|sc| Arc::new(StateCenterAdapter::new(sc)) as Arc<dyn StateCenterPort>);
        let events_port: Arc<dyn EventsPort> = if let Some(bus) = events_bus {
            Arc::new(BusEventsPort::new(bus)) as Arc<dyn EventsPort>
        } else {
            Arc::new(NoopEventsPort::default()) as Arc<dyn EventsPort>
        };
        let policy_port: Arc<dyn PolicyPort> =
            Arc::new(policy_handle.clone()) as Arc<dyn PolicyPort>;
        let service = Self::new(
            event_store_port,
            state_center_port,
            policy_port,
            events_port,
        );
        (service, policy_handle)
    }

    fn digest_request(&self, req: &ExportReq) -> QueryDigest {
        QueryDigest::from_req(req)
    }

    fn policy_snapshot_json(&self, policy: &TimelinePolicyView) -> JsonlLine {
        let view = to_value(policy).unwrap_or_default();
        build_header(view)
    }
}

#[async_trait]
impl Timeline for TimelineService {
    async fn export(&self, req: ExportReq) -> TlResult<ExportResult> {
        let digest = self.digest_request(&req);
        self.events.timeline_export_started(&digest);

        let started_at = Instant::now();
        let policy = self.policy.view();
        let hot_hint = self.event_store.hot_window_hint().await.ok();

        let plan = match build_plan(&req, &policy, hot_hint) {
            Ok(plan) => plan,
            Err(err) => {
                self.events.timeline_export_finished(
                    false,
                    started_at.elapsed().as_millis() as u128,
                    Some(&err),
                );
                return Err(err);
            }
        };

        let fetch_res = run_fetch(
            self.event_store.as_ref(),
            self.state_center.as_deref(),
            &plan,
        )
        .await;

        let outcome = match fetch_res {
            Ok(outcome) => outcome,
            Err(err) => {
                self.events.timeline_export_finished(
                    false,
                    started_at.elapsed().as_millis() as u128,
                    Some(&err),
                );
                return Err(err);
            }
        };

        let merged = merge_outcome(outcome);
        let source_name = describe_source(&plan);
        self.events
            .timeline_export_fetched(merged.len(), source_name.as_ref());

        let BuildOutput {
            mut lines,
            mut stats,
        } = build_lines(merged, &plan.view, &policy);

        if lines.len() > policy.max_lines {
            lines.truncate(policy.max_lines);
            stats.total_lines = policy.max_lines;
            stats.truncated = true;
        }

        let mut final_lines = Vec::with_capacity(lines.len() + 2);
        final_lines.push(self.policy_snapshot_json(&policy));
        final_lines.extend(lines);
        final_lines.push(build_footer(&stats));

        let serialized = match serialize_lines(&final_lines, policy.max_payload_bytes) {
            Ok(s) => s,
            Err(err) => {
                self.events.timeline_export_finished(
                    false,
                    started_at.elapsed().as_millis() as u128,
                    Some(&err),
                );
                return Err(err);
            }
        };

        let mut result = ExportResult {
            path: None,
            lines: None,
            stats,
        };

        if policy.log_enable {
            match write_lines(&policy.log_path, &serialized) {
                Ok(path) => result.path = Some(path),
                Err(err) => {
                    self.events.timeline_export_finished(
                        false,
                        started_at.elapsed().as_millis() as u128,
                        Some(&err),
                    );
                    return Err(err);
                }
            }
        } else {
            result.lines = Some(serialized);
        }

        self.events
            .timeline_export_finished(true, started_at.elapsed().as_millis() as u128, None);
        Ok(result)
    }

    async fn build_replay(&self, action_id: &str) -> TlResult<ReplayBundle> {
        if action_id.is_empty() {
            return Err(TlError::InvalidArg("action_id must not be empty".into()));
        }
        self.event_store.replay_minimal(action_id).await
    }

    fn policy_view(&self) -> TimelinePolicyView {
        self.policy.view()
    }
}
