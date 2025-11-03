use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::to_writer_pretty;
use soulbrowser_core_types::{ActionId, ExecRoute, FrameId, PageId, SessionId, SoulError};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Execution outcome for a dispatched tool call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchStatus {
    Success,
    Failure,
}

/// Summary collected for a finished dispatch attempt.
#[derive(Clone, Debug)]
pub struct DispatchEvent {
    pub action_id: ActionId,
    pub task_id: Option<String>,
    pub status: DispatchStatus,
    pub route: ExecRoute,
    pub tool: String,
    pub mutex_key: String,
    pub attempts: u32,
    pub wait_ms: u64,
    pub run_ms: u64,
    pub pending: usize,
    pub slots_available: usize,
    pub error: Option<SoulError>,
    pub output: Option<serde_json::Value>,
    pub recorded_at: std::time::SystemTime,
}

impl DispatchEvent {
    pub fn success(
        action_id: ActionId,
        task_id: Option<String>,
        route: ExecRoute,
        tool: String,
        mutex_key: String,
        attempts: u32,
        wait_ms: u64,
        run_ms: u64,
        pending: usize,
        slots_available: usize,
        output: Option<serde_json::Value>,
    ) -> Self {
        Self {
            action_id,
            task_id,
            status: DispatchStatus::Success,
            route,
            tool,
            mutex_key,
            attempts,
            wait_ms,
            run_ms,
            pending,
            slots_available,
            error: None,
            output,
            recorded_at: std::time::SystemTime::now(),
        }
    }

    pub fn failure(
        action_id: ActionId,
        task_id: Option<String>,
        route: ExecRoute,
        tool: String,
        mutex_key: String,
        attempts: u32,
        wait_ms: u64,
        run_ms: u64,
        pending: usize,
        slots_available: usize,
        error: SoulError,
        output: Option<serde_json::Value>,
    ) -> Self {
        Self {
            action_id,
            task_id,
            status: DispatchStatus::Failure,
            route,
            tool,
            mutex_key,
            attempts,
            wait_ms,
            run_ms,
            pending,
            slots_available,
            error: Some(error),
            output,
            recorded_at: std::time::SystemTime::now(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PerceiverEvent {
    pub route: ExecRoute,
    pub kind: PerceiverEventKind,
    pub recorded_at: std::time::SystemTime,
}

impl PerceiverEvent {
    pub fn resolve(
        route: ExecRoute,
        strategy: String,
        score: f32,
        candidate_count: usize,
        cache_hit: bool,
        breakdown: Vec<ScoreComponentRecord>,
        reason: String,
    ) -> Self {
        Self {
            route,
            kind: PerceiverEventKind::Resolve {
                strategy,
                score,
                candidate_count,
                cache_hit,
                breakdown,
                reason,
            },
            recorded_at: std::time::SystemTime::now(),
        }
    }

    pub fn judge(
        route: ExecRoute,
        check: String,
        ok: bool,
        reason: String,
        facts: serde_json::Value,
    ) -> Self {
        Self {
            route,
            kind: PerceiverEventKind::Judge {
                check,
                ok,
                reason,
                facts,
            },
            recorded_at: std::time::SystemTime::now(),
        }
    }

    pub fn snapshot(route: ExecRoute, cache_hit: bool) -> Self {
        Self {
            route,
            kind: PerceiverEventKind::Snapshot { cache_hit },
            recorded_at: std::time::SystemTime::now(),
        }
    }

    pub fn diff(route: ExecRoute, change_count: usize, changes: Vec<serde_json::Value>) -> Self {
        Self {
            route,
            kind: PerceiverEventKind::Diff {
                change_count,
                changes,
            },
            recorded_at: std::time::SystemTime::now(),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ScoreComponentRecord {
    pub label: String,
    pub weight: f32,
    pub contribution: f32,
}

#[derive(Clone, Debug)]
pub enum PerceiverEventKind {
    Resolve {
        strategy: String,
        score: f32,
        candidate_count: usize,
        cache_hit: bool,
        breakdown: Vec<ScoreComponentRecord>,
        reason: String,
    },
    Judge {
        check: String,
        ok: bool,
        reason: String,
        facts: serde_json::Value,
    },
    Snapshot {
        cache_hit: bool,
    },
    Diff {
        change_count: usize,
        changes: Vec<serde_json::Value>,
    },
}

#[derive(Clone, Debug)]
pub enum StateEvent {
    Dispatch(DispatchEvent),
    Registry(RegistryEvent),
    Perceiver(PerceiverEvent),
}

impl StateEvent {
    pub fn dispatch_success(event: DispatchEvent) -> Self {
        Self::Dispatch(event)
    }

    pub fn dispatch_failure(event: DispatchEvent) -> Self {
        Self::Dispatch(event)
    }

    pub fn registry(event: RegistryEvent) -> Self {
        Self::Registry(event)
    }

    pub fn perceiver(event: PerceiverEvent) -> Self {
        Self::Perceiver(event)
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StateCenterStats {
    pub total_events: u64,
    pub dispatch_success: u64,
    pub dispatch_failure: u64,
    pub registry_events: u64,
    pub perceiver_resolve: u64,
    pub perceiver_judge: u64,
    pub perceiver_snapshot: u64,
    pub perceiver_diff: u64,
}

#[derive(Debug)]
struct BoundedRing<T> {
    capacity: usize,
    data: VecDeque<T>,
}

impl<T> BoundedRing<T> {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            data: VecDeque::new(),
        }
    }
}

impl<T: Clone> BoundedRing<T> {
    fn push(&mut self, item: T) {
        if self.data.len() == self.capacity {
            self.data.pop_front();
        }
        self.data.push_back(item);
    }

    fn snapshot(&self) -> Vec<T> {
        self.data.iter().cloned().collect()
    }

    fn len(&self) -> usize {
        self.data.len()
    }
}

#[async_trait]
pub trait StateCenter: Send + Sync {
    async fn append(&self, event: StateEvent) -> Result<(), SoulError>;
}

/// In-memory ring buffer storing recent dispatch events for diagnostics.
pub struct InMemoryStateCenter {
    #[allow(dead_code)]
    global_capacity: usize,
    session_capacity: usize,
    page_capacity: usize,
    frame_capacity: usize,
    task_capacity: usize,
    action_capacity: usize,
    events: Mutex<BoundedRing<StateEvent>>,
    session_events: DashMap<SessionId, Mutex<BoundedRing<StateEvent>>>,
    page_events: DashMap<PageId, Mutex<BoundedRing<StateEvent>>>,
    frame_events: DashMap<FrameId, Mutex<BoundedRing<StateEvent>>>,
    task_events: DashMap<String, Mutex<BoundedRing<StateEvent>>>,
    action_events: DashMap<String, Mutex<BoundedRing<StateEvent>>>,
    stats: Mutex<StateCenterStats>,
}

impl InMemoryStateCenter {
    pub fn new(capacity: usize) -> Self {
        let global_capacity = capacity.max(1);
        let session_capacity = std::cmp::max(global_capacity / 2, 32);
        let page_capacity = std::cmp::max(global_capacity / 2, 32);
        let frame_capacity = std::cmp::max(global_capacity / 2, 32);
        let task_capacity = std::cmp::max(global_capacity / 4, 16);
        let action_capacity = std::cmp::max(global_capacity / 4, 16);
        Self {
            global_capacity,
            session_capacity,
            page_capacity,
            frame_capacity,
            task_capacity,
            action_capacity,
            events: Mutex::new(BoundedRing::new(global_capacity)),
            session_events: DashMap::new(),
            page_events: DashMap::new(),
            frame_events: DashMap::new(),
            task_events: DashMap::new(),
            action_events: DashMap::new(),
            stats: Mutex::new(StateCenterStats::default()),
        }
    }

    pub fn snapshot(&self) -> Vec<StateEvent> {
        self.events.lock().snapshot()
    }

    pub fn stats(&self) -> StateCenterStats {
        self.stats.lock().clone()
    }

    pub fn recent_session(&self, session: &SessionId) -> Vec<StateEvent> {
        self.session_events
            .get(session)
            .map(|entry| entry.value().lock().snapshot())
            .unwrap_or_default()
    }

    pub fn recent_page(&self, page: &PageId) -> Vec<StateEvent> {
        self.page_events
            .get(page)
            .map(|entry| entry.value().lock().snapshot())
            .unwrap_or_default()
    }

    pub fn recent_frame(&self, frame: &FrameId) -> Vec<StateEvent> {
        self.frame_events
            .get(frame)
            .map(|entry| entry.value().lock().snapshot())
            .unwrap_or_default()
    }

    pub fn recent_task(&self, task_id: &str) -> Vec<StateEvent> {
        self.task_events
            .get(task_id)
            .map(|entry| entry.value().lock().snapshot())
            .unwrap_or_default()
    }

    pub fn recent_action(&self, action_id: &str) -> Vec<StateEvent> {
        self.action_events
            .get(action_id)
            .map(|entry| entry.value().lock().snapshot())
            .unwrap_or_default()
    }

    pub fn write_snapshot<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let events = self.snapshot();
        let stats = self.stats();
        let serialized_events: Vec<SerializableStateEvent> =
            events.iter().map(SerializableStateEvent::from).collect();
        let snapshot = StateCenterSnapshot {
            stats,
            events: serialized_events,
            scopes: self.scope_counters(),
        };
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        to_writer_pretty(&mut writer, &snapshot)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        writer.flush()?;
        Ok(())
    }
}

#[async_trait]
impl StateCenter for InMemoryStateCenter {
    async fn append(&self, event: StateEvent) -> Result<(), SoulError> {
        {
            let mut guard = self.events.lock();
            guard.push(event.clone());
        }
        self.push_scoped(&event);
        self.update_stats(&event);
        Ok(())
    }
}

/// No-op state center for tests and benchmarks.
pub struct NoopStateCenter;

impl NoopStateCenter {
    pub fn new() -> Arc<dyn StateCenter> {
        Arc::new(Self)
    }
}

#[async_trait]
impl StateCenter for NoopStateCenter {
    async fn append(&self, _event: StateEvent) -> Result<(), SoulError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;
    use soulbrowser_core_types::{ActionId, FrameId, PageId, SessionId};
    use tempfile::NamedTempFile;

    fn mock_route() -> ExecRoute {
        ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new())
    }

    #[tokio::test]
    async fn in_memory_center_bounded() {
        let center = InMemoryStateCenter::new(2);
        let base_route = mock_route();
        let session_id = base_route.session.clone();
        let page_id = base_route.page.clone();
        let frame_id = base_route.frame.clone();
        let action_success = ActionId::new();
        let action_failure = ActionId::new();

        center
            .append(StateEvent::dispatch_success(DispatchEvent::success(
                action_success.clone(),
                Some("task-1".into()),
                base_route.clone(),
                "tool".into(),
                "mutex".into(),
                1,
                10,
                20,
                0,
                4,
                None,
            )))
            .await
            .unwrap();

        center
            .append(StateEvent::dispatch_failure(DispatchEvent::failure(
                action_failure.clone(),
                Some("task-1".into()),
                base_route.clone(),
                "tool".into(),
                "mutex".into(),
                2,
                15,
                25,
                1,
                3,
                SoulError::new("fail"),
                None,
            )))
            .await
            .unwrap();

        center
            .append(StateEvent::dispatch_success(DispatchEvent::success(
                ActionId::new(),
                Some("task-2".into()),
                base_route.clone(),
                "tool".into(),
                "mutex".into(),
                1,
                5,
                30,
                2,
                2,
                None,
            )))
            .await
            .unwrap();

        center
            .append(StateEvent::registry(RegistryEvent::new(
                RegistryAction::PageOpened,
                None,
                None,
                None,
                Some("test".into()),
            )))
            .await
            .unwrap();

        let events = center.snapshot();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], StateEvent::Dispatch(_)));
        assert!(matches!(events[1], StateEvent::Registry(_)));
        let stats = center.stats();
        assert_eq!(stats.total_events, 4);
        assert_eq!(stats.registry_events, 1);
        assert_eq!(stats.dispatch_success, 2);
        assert_eq!(stats.dispatch_failure, 1);

        let session_recent = center.recent_session(&session_id);
        assert!(!session_recent.is_empty());
        let page_recent = center.recent_page(&page_id);
        assert!(!page_recent.is_empty());
        let frame_recent = center.recent_frame(&frame_id);
        assert!(!frame_recent.is_empty());
        let task_recent = center.recent_task("task-1");
        assert_eq!(task_recent.len(), 2);
        let action_recent = center.recent_action(&action_success.0);
        assert_eq!(action_recent.len(), 1);

        let file = NamedTempFile::new().expect("tempfile");
        center
            .write_snapshot(file.path())
            .expect("write snapshot to disk");
        let written = std::fs::read_to_string(file.path()).expect("read snapshot");
        assert!(written.contains("\"total_events\""));
        assert!(written.contains("dispatch_success"));
        assert!(written.contains("\"scopes\""));
    }

    #[tokio::test]
    async fn perceiver_events_recorded() {
        let center = InMemoryStateCenter::new(8);
        let route = mock_route();

        center
            .append(StateEvent::perceiver(PerceiverEvent::resolve(
                route.clone(),
                "css".into(),
                0.82,
                3,
                false,
                vec![ScoreComponentRecord {
                    label: "confidence".into(),
                    weight: 1.0,
                    contribution: 0.82,
                }],
                "score=0.82".into(),
            )))
            .await
            .unwrap();

        center
            .append(StateEvent::perceiver(PerceiverEvent::judge(
                route.clone(),
                "clickable".into(),
                true,
                "visible".into(),
                json!({ "geometry": {"width": 120} }),
            )))
            .await
            .unwrap();

        center
            .append(StateEvent::perceiver(PerceiverEvent::snapshot(
                route.clone(),
                true,
            )))
            .await
            .unwrap();

        center
            .append(StateEvent::perceiver(PerceiverEvent::diff(
                route.clone(),
                4,
                vec![json!({"kind": "test"})],
            )))
            .await
            .unwrap();

        let stats = center.stats();
        assert_eq!(stats.perceiver_resolve, 1);
        assert_eq!(stats.perceiver_judge, 1);
        assert_eq!(stats.perceiver_snapshot, 1);
        assert_eq!(stats.perceiver_diff, 1);

        let session_events = center.recent_session(&route.session);
        assert_eq!(session_events.len(), 4);
        assert!(session_events
            .iter()
            .all(|event| matches!(event, StateEvent::Perceiver(_))));

        let frame_events = center.recent_frame(&route.frame);
        assert_eq!(frame_events.len(), 4);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryAction {
    SessionCreated,
    SessionClosed,
    PageOpened,
    PageClosed,
    PageFocused,
    FrameFocused,
    FrameAttached,
    FrameDetached,
    HealthProbeTick,
    PageHealthUpdated,
    PermissionsApplied,
}

#[derive(Serialize)]
struct StateCenterSnapshot {
    stats: StateCenterStats,
    events: Vec<SerializableStateEvent>,
    scopes: ScopeCounters,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "data")]
enum SerializableStateEvent {
    Dispatch(SerializableDispatchEvent),
    Registry(SerializableRegistryEvent),
    Perceiver(SerializablePerceiverEvent),
}

#[derive(Serialize)]
struct SerializableDispatchEvent {
    status: &'static str,
    action_id: String,
    task_id: Option<String>,
    route: RouteSnapshot,
    tool: String,
    mutex_key: String,
    attempts: u32,
    wait_ms: u64,
    run_ms: u64,
    pending: usize,
    slots_available: usize,
    error: Option<String>,
    recorded_at_ms: u128,
}

#[derive(Serialize)]
struct SerializableRegistryEvent {
    action: String,
    session: Option<String>,
    page: Option<String>,
    frame: Option<String>,
    note: Option<String>,
    recorded_at_ms: u128,
}

#[derive(Serialize)]
struct SerializablePerceiverEvent {
    phase: &'static str,
    route: RouteSnapshot,
    recorded_at_ms: u128,
    strategy: Option<String>,
    score: Option<f32>,
    candidate_count: Option<usize>,
    cache_hit: Option<bool>,
    check: Option<String>,
    ok: Option<bool>,
    reason: Option<String>,
    change_count: Option<usize>,
    score_breakdown: Option<Vec<ScoreComponentRecord>>,
    facts: Option<serde_json::Value>,
    changes: Option<Vec<serde_json::Value>>,
}

#[derive(Serialize)]
struct RouteSnapshot {
    session: String,
    page: String,
    frame: String,
    mutex_key: String,
}

#[derive(Serialize, Default)]
struct ScopeCounters {
    sessions: Vec<ScopeCount>,
    pages: Vec<ScopeCount>,
    frames: Vec<ScopeCount>,
    tasks: Vec<ScopeCount>,
    actions: Vec<ScopeCount>,
}

#[derive(Serialize)]
struct ScopeCount {
    id: String,
    count: usize,
}

impl InMemoryStateCenter {
    fn push_scoped(&self, event: &StateEvent) {
        match event {
            StateEvent::Dispatch(dispatch) => {
                self.push_session_event(&dispatch.route.session, event);
                self.push_page_event(&dispatch.route.page, event);
                self.push_frame_event(&dispatch.route.frame, event);
                if let Some(task_id) = dispatch.task_id.as_ref() {
                    self.push_task_event(task_id, event);
                }
                self.push_action_event(&dispatch.action_id.0, event);
            }
            StateEvent::Registry(registry) => {
                if let Some(session) = registry.session.as_ref() {
                    self.push_session_event(session, event);
                }
                if let Some(page) = registry.page.as_ref() {
                    self.push_page_event(page, event);
                }
                if let Some(frame) = registry.frame.as_ref() {
                    self.push_frame_event(frame, event);
                }
            }
            StateEvent::Perceiver(perceiver) => {
                self.push_session_event(&perceiver.route.session, event);
                self.push_page_event(&perceiver.route.page, event);
                self.push_frame_event(&perceiver.route.frame, event);
            }
        }
    }

    fn push_session_event(&self, session: &SessionId, event: &StateEvent) {
        let mut entry = self
            .session_events
            .entry(session.clone())
            .or_insert_with(|| Mutex::new(BoundedRing::new(self.session_capacity)));
        entry.value_mut().lock().push(event.clone());
    }

    fn push_page_event(&self, page: &PageId, event: &StateEvent) {
        let mut entry = self
            .page_events
            .entry(page.clone())
            .or_insert_with(|| Mutex::new(BoundedRing::new(self.page_capacity)));
        entry.value_mut().lock().push(event.clone());
    }

    fn push_frame_event(&self, frame: &FrameId, event: &StateEvent) {
        let mut entry = self
            .frame_events
            .entry(frame.clone())
            .or_insert_with(|| Mutex::new(BoundedRing::new(self.frame_capacity)));
        entry.value_mut().lock().push(event.clone());
    }

    fn push_task_event(&self, task_id: &str, event: &StateEvent) {
        let mut entry = self
            .task_events
            .entry(task_id.to_string())
            .or_insert_with(|| Mutex::new(BoundedRing::new(self.task_capacity)));
        entry.value_mut().lock().push(event.clone());
    }

    fn push_action_event(&self, action_id: &str, event: &StateEvent) {
        let mut entry = self
            .action_events
            .entry(action_id.to_string())
            .or_insert_with(|| Mutex::new(BoundedRing::new(self.action_capacity)));
        entry.value_mut().lock().push(event.clone());
    }

    fn scope_counters(&self) -> ScopeCounters {
        ScopeCounters {
            sessions: self
                .session_events
                .iter()
                .map(|entry| ScopeCount {
                    id: entry.key().0.clone(),
                    count: entry.value().lock().len(),
                })
                .collect(),
            pages: self
                .page_events
                .iter()
                .map(|entry| ScopeCount {
                    id: entry.key().0.clone(),
                    count: entry.value().lock().len(),
                })
                .collect(),
            frames: self
                .frame_events
                .iter()
                .map(|entry| ScopeCount {
                    id: entry.key().0.clone(),
                    count: entry.value().lock().len(),
                })
                .collect(),
            tasks: self
                .task_events
                .iter()
                .map(|entry| ScopeCount {
                    id: entry.key().clone(),
                    count: entry.value().lock().len(),
                })
                .collect(),
            actions: self
                .action_events
                .iter()
                .map(|entry| ScopeCount {
                    id: entry.key().clone(),
                    count: entry.value().lock().len(),
                })
                .collect(),
        }
    }

    fn update_stats(&self, event: &StateEvent) {
        let mut stats = self.stats.lock();
        stats.total_events = stats.total_events.saturating_add(1);
        match event {
            StateEvent::Dispatch(dispatch) => match dispatch.status {
                DispatchStatus::Success => {
                    stats.dispatch_success = stats.dispatch_success.saturating_add(1)
                }
                DispatchStatus::Failure => {
                    stats.dispatch_failure = stats.dispatch_failure.saturating_add(1)
                }
            },
            StateEvent::Registry(_) => {
                stats.registry_events = stats.registry_events.saturating_add(1)
            }
            StateEvent::Perceiver(perceiver) => match &perceiver.kind {
                PerceiverEventKind::Resolve { .. } => {
                    stats.perceiver_resolve = stats.perceiver_resolve.saturating_add(1)
                }
                PerceiverEventKind::Judge { .. } => {
                    stats.perceiver_judge = stats.perceiver_judge.saturating_add(1)
                }
                PerceiverEventKind::Snapshot { .. } => {
                    stats.perceiver_snapshot = stats.perceiver_snapshot.saturating_add(1)
                }
                PerceiverEventKind::Diff { .. } => {
                    stats.perceiver_diff = stats.perceiver_diff.saturating_add(1)
                }
            },
        }
    }
}

impl From<&StateEvent> for SerializableStateEvent {
    fn from(value: &StateEvent) -> Self {
        match value {
            StateEvent::Dispatch(event) => SerializableStateEvent::Dispatch(event.into()),
            StateEvent::Registry(event) => SerializableStateEvent::Registry(event.into()),
            StateEvent::Perceiver(event) => SerializableStateEvent::Perceiver(event.into()),
        }
    }
}

impl From<&DispatchEvent> for SerializableDispatchEvent {
    fn from(event: &DispatchEvent) -> Self {
        Self {
            status: match event.status {
                DispatchStatus::Success => "success",
                DispatchStatus::Failure => "failure",
            },
            action_id: event.action_id.0.clone(),
            task_id: event.task_id.clone(),
            route: RouteSnapshot {
                session: event.route.session.0.clone(),
                page: event.route.page.0.clone(),
                frame: event.route.frame.0.clone(),
                mutex_key: event.route.mutex_key.clone(),
            },
            tool: event.tool.clone(),
            mutex_key: event.mutex_key.clone(),
            attempts: event.attempts,
            wait_ms: event.wait_ms,
            run_ms: event.run_ms,
            pending: event.pending,
            slots_available: event.slots_available,
            error: event.error.as_ref().map(|err| err.to_string()),
            recorded_at_ms: timestamp_ms(event.recorded_at),
        }
    }
}

impl From<&RegistryEvent> for SerializableRegistryEvent {
    fn from(event: &RegistryEvent) -> Self {
        Self {
            action: format!("{:?}", event.action),
            session: event.session.as_ref().map(|s| s.0.clone()),
            page: event.page.as_ref().map(|p| p.0.clone()),
            frame: event.frame.as_ref().map(|f| f.0.clone()),
            note: event.note.clone(),
            recorded_at_ms: timestamp_ms(event.recorded_at),
        }
    }
}

impl From<&PerceiverEvent> for SerializablePerceiverEvent {
    fn from(event: &PerceiverEvent) -> Self {
        let mut strategy = None;
        let mut score = None;
        let mut candidate_count = None;
        let mut cache_hit = None;
        let mut check = None;
        let mut ok = None;
        let mut reason = None;
        let mut change_count = None;
        let mut breakdown = None;
        let mut facts = None;
        let mut changes_detail = None;
        let phase = match &event.kind {
            PerceiverEventKind::Resolve {
                strategy: strat,
                score: s,
                candidate_count: count,
                cache_hit: cache,
                breakdown: components,
                reason: r,
            } => {
                strategy = Some(strat.clone());
                score = Some(*s);
                candidate_count = Some(*count);
                cache_hit = Some(*cache);
                breakdown = Some(components.clone());
                reason = Some(r.clone());
                "resolve"
            }
            PerceiverEventKind::Judge {
                check: chk,
                ok: success,
                reason: msg,
                facts: payload,
            } => {
                check = Some(chk.clone());
                ok = Some(*success);
                reason = Some(msg.clone());
                facts = Some(payload.clone());
                "judge"
            }
            PerceiverEventKind::Snapshot { cache_hit: cache } => {
                cache_hit = Some(*cache);
                "snapshot"
            }
            PerceiverEventKind::Diff {
                change_count: changes,
                changes: detail,
            } => {
                change_count = Some(*changes);
                changes_detail = Some(detail.clone());
                "diff"
            }
        };

        Self {
            phase,
            route: RouteSnapshot {
                session: event.route.session.0.clone(),
                page: event.route.page.0.clone(),
                frame: event.route.frame.0.clone(),
                mutex_key: event.route.mutex_key.clone(),
            },
            recorded_at_ms: timestamp_ms(event.recorded_at),
            strategy,
            score,
            candidate_count,
            cache_hit,
            check,
            ok,
            reason,
            change_count,
            score_breakdown: breakdown,
            facts,
            changes: changes_detail,
        }
    }
}

fn timestamp_ms(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .map(|dur| dur.as_millis())
        .unwrap_or(0)
}

#[derive(Clone, Debug)]
pub struct RegistryEvent {
    pub action: RegistryAction,
    pub session: Option<SessionId>,
    pub page: Option<PageId>,
    pub frame: Option<FrameId>,
    pub note: Option<String>,
    pub recorded_at: std::time::SystemTime,
}

impl RegistryEvent {
    pub fn new(
        action: RegistryAction,
        session: Option<SessionId>,
        page: Option<PageId>,
        frame: Option<FrameId>,
        note: Option<String>,
    ) -> Self {
        Self {
            action,
            session,
            page,
            frame,
            note,
            recorded_at: std::time::SystemTime::now(),
        }
    }
}
