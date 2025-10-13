use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use dashmap::{DashMap, DashSet};
use parking_lot::{Mutex, RwLock};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};
use tracing::warn;

use soulbrowser_core_types::{ActionId, ExecRoute};

use crate::lane::{Job, PriorityLane};
use crate::model::{DispatchOutput, DispatchRequest, DispatchTimeline, Priority, SchedulerConfig};

#[derive(Debug)]
pub struct JobEntry {
    pub id: ActionId,
    pub mutex_key: String,
    pub request: DispatchRequest,
    pub route: ExecRoute,
    pub job: Job,
    pub task_id: Option<String>,
    pub timeline: Mutex<DispatchTimeline>,
    pub completion: Mutex<Option<oneshot::Sender<DispatchOutput>>>,
}

#[derive(Debug)]
pub struct ReadyJob {
    entry: Arc<JobEntry>,
    pub permit: OwnedSemaphorePermit,
}

impl ReadyJob {
    pub fn id(&self) -> ActionId {
        self.entry.id.clone()
    }

    pub fn mutex_key(&self) -> String {
        self.entry.mutex_key.clone()
    }

    pub fn request(&self) -> &DispatchRequest {
        &self.entry.request
    }

    pub fn route(&self) -> &ExecRoute {
        &self.entry.route
    }

    pub fn job(&self) -> Job {
        self.entry.job.clone()
    }

    pub fn task_id(&self) -> Option<String> {
        self.entry.task_id.clone()
    }

    pub fn mark_started(&self) -> u64 {
        let mut timeline = self.entry.timeline.lock();
        let now = std::time::Instant::now();
        let wait_ms = now
            .checked_duration_since(timeline.enqueued_at)
            .map(|dur| dur.as_millis() as u64)
            .unwrap_or(0);
        timeline.started_at = Some(now);
        wait_ms
    }

    pub fn take_completion(&self) -> Option<oneshot::Sender<DispatchOutput>> {
        self.entry.completion.lock().take()
    }

    pub fn mark_finished(&self) -> DispatchTimeline {
        let mut timeline = self.entry.timeline.lock();
        timeline.finished_at = Some(std::time::Instant::now());
        timeline.clone()
    }
}

#[derive(Debug)]
pub struct LaneManager {
    lanes: DashMap<String, Arc<Mutex<PriorityLane>>>,
    order: Mutex<Vec<String>>,
    cursor: AtomicUsize,
    seq: AtomicU64,
    weights: [u8; 4],
}

impl LaneManager {
    pub fn new(weights: [u8; 4]) -> Self {
        Self {
            lanes: DashMap::new(),
            order: Mutex::new(Vec::new()),
            cursor: AtomicUsize::new(0),
            seq: AtomicU64::new(0),
            weights,
        }
    }

    pub fn enqueue(&self, key: impl Into<String>, priority: Priority) -> Job {
        let key = key.into();
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let job = Job::new(priority, seq);
        let lane = self
            .lanes
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(PriorityLane::new(self.weights))))
            .clone();

        {
            let mut guard = lane.lock();
            guard.push(job.clone());
        }

        let mut order = self.order.lock();
        if !order.iter().any(|existing| existing == &key) {
            order.push(key);
        }

        job
    }

    pub fn requeue(&self, key: String, job: Job) {
        let lane = self
            .lanes
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(PriorityLane::new(self.weights))))
            .clone();
        {
            let mut guard = lane.lock();
            guard.push(job);
        }
        let mut order = self.order.lock();
        if !order.iter().any(|existing| existing == &key) {
            order.push(key);
        }
    }

    pub fn dequeue(&self) -> Option<(String, Job)> {
        let mut order = self.order.lock();
        if order.is_empty() {
            return None;
        }

        let mut len = order.len();
        let mut idx = self.cursor.load(Ordering::Relaxed) % len;
        let mut traversed = 0;

        while traversed < len {
            let key = order[idx].clone();
            if let Some(lane_entry) = self.lanes.get(&key) {
                let mut guard = lane_entry.lock();
                if let Some(job) = guard.pop() {
                    let empty = guard.is_empty();
                    drop(guard);
                    drop(lane_entry);

                    if empty {
                        self.lanes.remove(&key);
                        order.remove(idx);
                        len = order.len();
                        if len == 0 {
                            self.cursor.store(0, Ordering::Relaxed);
                        } else if idx >= len {
                            self.cursor.store(0, Ordering::Relaxed);
                        } else {
                            self.cursor.store(idx, Ordering::Relaxed);
                        }
                    } else {
                        self.cursor.store((idx + 1) % len.max(1), Ordering::Relaxed);
                    }
                    return Some((key, job));
                }
            } else {
                order.remove(idx);
                len = order.len();
                if len == 0 {
                    self.cursor.store(0, Ordering::Relaxed);
                    return None;
                }
                if idx >= len {
                    idx = 0;
                }
                continue;
            }

            traversed += 1;
            idx = (idx + 1) % len;
        }

        None
    }

    pub fn is_empty(&self) -> bool {
        self.order.lock().is_empty()
    }
}

#[derive(Debug)]
pub struct SchedulerRuntime {
    lanes: LaneManager,
    jobs: DashMap<ActionId, Arc<JobEntry>>,
    global_slots: Arc<Semaphore>,
    per_task: DashMap<String, Arc<AtomicUsize>>,
    config: RwLock<SchedulerConfig>,
    global_slots_limit: AtomicUsize,
    per_task_limit: AtomicUsize,
    call_index: DashMap<String, ActionId>,
    task_index: DashMap<String, DashSet<ActionId>>,
}

impl SchedulerRuntime {
    pub fn new(config: SchedulerConfig) -> Self {
        let weights = [8, 4, 2, 1];
        let global_slots_limit = AtomicUsize::new(config.global_slots);
        let per_task_limit = AtomicUsize::new(config.per_task_limit);
        Self {
            lanes: LaneManager::new(weights),
            jobs: DashMap::new(),
            global_slots: Arc::new(Semaphore::new(config.global_slots)),
            per_task: DashMap::new(),
            config: RwLock::new(config),
            global_slots_limit,
            per_task_limit,
            call_index: DashMap::new(),
            task_index: DashMap::new(),
        }
    }

    pub fn enqueue(
        &self,
        mutex_key: impl Into<String>,
        request: DispatchRequest,
        route: ExecRoute,
        completion: oneshot::Sender<DispatchOutput>,
    ) -> ActionId {
        let key = mutex_key.into();
        let priority = request.options.priority;
        let job = self.lanes.enqueue(key.clone(), priority);
        let task_id = request.tool_call.task_id.as_ref().map(|tid| tid.0.clone());
        let entry = Arc::new(JobEntry {
            id: job.id.clone(),
            mutex_key: key.clone(),
            request,
            route,
            job,
            task_id,
            timeline: Mutex::new(DispatchTimeline::default()),
            completion: Mutex::new(Some(completion)),
        });
        let id = entry.id.clone();
        self.register_indices(&entry);
        self.jobs.insert(id.clone(), entry);
        id
    }

    pub async fn next_job(&self) -> Option<ReadyJob> {
        loop {
            let (_key, job) = self.lanes.dequeue()?;
            if let Some((_, entry)) = self.jobs.remove(&job.id) {
                if let Some(task_id) = entry.task_id.as_ref() {
                    if !self.acquire_task_slot(task_id) {
                        self.jobs.insert(entry.id.clone(), Arc::clone(&entry));
                        self.lanes
                            .requeue(entry.mutex_key.clone(), entry.job.clone());
                        continue;
                    }
                }

                let permit = match self.global_slots.clone().acquire_owned().await {
                    Ok(permit) => permit,
                    Err(_) => {
                        if let Some(task_id) = entry.task_id.as_ref() {
                            self.release_task_slot(task_id);
                        }
                        self.jobs.insert(entry.id.clone(), Arc::clone(&entry));
                        self.lanes
                            .requeue(entry.mutex_key.clone(), entry.job.clone());
                        continue;
                    }
                };
                self.unregister_indices(&entry);

                let ready = ReadyJob {
                    entry: Arc::clone(&entry),
                    permit,
                };
                ready.mark_started();
                return Some(ready);
            }
        }
    }

    pub fn global_slots(&self) -> Arc<Semaphore> {
        Arc::clone(&self.global_slots)
    }

    pub fn pending(&self) -> usize {
        self.jobs.len()
    }

    pub fn config(&self) -> SchedulerConfig {
        self.config.read().clone()
    }

    fn acquire_task_slot(&self, task_id: &str) -> bool {
        let counter = self
            .per_task
            .entry(task_id.to_string())
            .or_insert_with(|| Arc::new(AtomicUsize::new(0)))
            .clone();

        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                if current >= self.per_task_limit.load(Ordering::Relaxed) {
                    None
                } else {
                    Some(current + 1)
                }
            })
            .is_ok()
    }

    fn release_task_slot(&self, task_id: &str) {
        if let Some(counter) = self.per_task.get(task_id) {
            counter.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub fn finish_job(&self, ready: ReadyJob) -> DispatchTimeline {
        if let Some(task_id) = ready.task_id() {
            self.release_task_slot(&task_id);
        }
        let timeline = ready.mark_finished();
        drop(ready);
        timeline
    }

    pub fn cancel(&self, id: &ActionId) -> Option<(DispatchRequest, ExecRoute)> {
        self.jobs.remove(id).map(|(_, entry)| {
            let request = entry.request.clone();
            let route = entry.route.clone();
            self.unregister_indices(&entry);
            (request, route)
        })
    }

    pub fn cancel_call(&self, call_id: &str) -> Option<(ActionId, DispatchRequest, ExecRoute)> {
        if let Some(action_entry) = self.call_index.get(call_id) {
            let action_id = action_entry.clone();
            drop(action_entry);
            if let Some((request, route)) = self.cancel(&action_id) {
                return Some((action_id, request, route));
            }
        }
        None
    }

    pub fn cancel_task(&self, task_id: &str) -> Vec<(ActionId, DispatchRequest, ExecRoute)> {
        let mut cancelled = Vec::new();
        if let Some(set_ref) = self.task_index.get(task_id) {
            let ids: Vec<ActionId> = set_ref.iter().map(|id| id.clone()).collect();
            drop(set_ref);
            for id in ids {
                if let Some((request, route)) = self.cancel(&id) {
                    cancelled.push((id.clone(), request, route));
                }
            }
        }
        self.task_index.remove(task_id);
        cancelled
    }

    pub fn update_config(&self, updated: SchedulerConfig) {
        let old_global = self.global_slots_limit.load(Ordering::Relaxed);
        if updated.global_slots > old_global {
            self.global_slots
                .add_permits(updated.global_slots - old_global);
        } else if updated.global_slots < old_global {
            warn!(
                old = old_global,
                new = updated.global_slots,
                "shrinking global slots is not fully supported yet; allowing in-flight permits"
            );
        }
        self.global_slots_limit
            .store(updated.global_slots, Ordering::Relaxed);
        self.per_task_limit
            .store(updated.per_task_limit, Ordering::Relaxed);
        *self.config.write() = updated;
    }

    fn register_indices(&self, entry: &Arc<JobEntry>) {
        if let Some(call_id) = entry.request.tool_call.call_id.clone() {
            self.call_index.insert(call_id, entry.id.clone());
        }
        if let Some(task_id) = entry.task_id.as_ref() {
            let set = self
                .task_index
                .entry(task_id.clone())
                .or_insert_with(DashSet::new);
            set.insert(entry.id.clone());
        }
    }

    fn unregister_indices(&self, entry: &Arc<JobEntry>) {
        if let Some(call_id) = entry.request.tool_call.call_id.as_ref() {
            if let Some(existing) = self.call_index.get(call_id) {
                if *existing == entry.id {
                    drop(existing);
                    self.call_index.remove(call_id);
                }
            }
        }
        if let Some(task_id) = entry.task_id.as_ref() {
            if let Some(set_ref) = self.task_index.get(task_id) {
                set_ref.remove(&entry.id);
                let empty = set_ref.is_empty();
                drop(set_ref);
                if empty {
                    self.task_index.remove(task_id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CallOptions, DispatchRequest, Priority};
    use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId, TaskId, ToolCall};
    use tokio::sync::oneshot;

    fn mock_request(priority: Priority, tool: &str) -> DispatchRequest {
        DispatchRequest {
            tool_call: ToolCall {
                tool: tool.to_string(),
                ..Default::default()
            },
            options: CallOptions {
                priority,
                ..CallOptions::default()
            },
            routing_hint: None,
        }
    }

    fn mock_route() -> ExecRoute {
        ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new())
    }

    #[tokio::test]
    async fn enqueue_returns_ids_and_next_job_yields_same() {
        let runtime = SchedulerRuntime::new(SchedulerConfig::default());
        let (tx, _rx) = oneshot::channel();
        let id = runtime.enqueue(
            "frame:a",
            mock_request(Priority::Standard, "click"),
            mock_route(),
            tx,
        );

        let ready = runtime.next_job().await.unwrap();
        assert_eq!(ready.id(), id);
        assert_eq!(ready.mutex_key(), "frame:a");
        assert_eq!(ready.request().tool_call.tool, "click");
        let finished = runtime.finish_job(ready);
        assert!(finished.started_at.is_some());
        assert!(finished.finished_at.is_some());
    }

    #[tokio::test]
    async fn jobs_interleave_across_mutex_keys() {
        let runtime = SchedulerRuntime::new(SchedulerConfig::default());
        let (tx1, _rx1) = oneshot::channel();
        runtime.enqueue(
            "frame:a",
            mock_request(Priority::Quick, "click"),
            mock_route(),
            tx1,
        );
        let (tx2, _rx2) = oneshot::channel();
        runtime.enqueue(
            "frame:b",
            mock_request(Priority::Quick, "type"),
            mock_route(),
            tx2,
        );
        let (tx3, _rx3) = oneshot::channel();
        runtime.enqueue(
            "frame:a",
            mock_request(Priority::Quick, "scroll"),
            mock_route(),
            tx3,
        );

        let first = runtime.next_job().await.unwrap();
        let first_key = first.mutex_key();
        runtime.finish_job(first);
        let second = runtime.next_job().await.unwrap();
        let second_key = second.mutex_key();
        runtime.finish_job(second);
        let third = runtime.next_job().await.unwrap();
        let third_key = third.mutex_key();
        runtime.finish_job(third);

        assert_ne!(first_key, second_key);
        assert!(third_key == "frame:a" || third_key == "frame:b");
        assert_eq!(runtime.pending(), 0);
    }

    #[tokio::test]
    async fn enforces_per_task_limit() {
        let mut config = SchedulerConfig::default();
        config.per_task_limit = 1;
        let runtime = SchedulerRuntime::new(config);

        let mut request = mock_request(Priority::Quick, "click");
        request.tool_call.task_id = Some(TaskId("task-1".to_string()));

        let (tx1, _rx1) = oneshot::channel();
        let first_id = runtime.enqueue("frame:a", request.clone(), mock_route(), tx1);
        let (tx2, _rx2) = oneshot::channel();
        runtime.enqueue("frame:b", request, mock_route(), tx2);

        let first = runtime.next_job().await.unwrap();
        assert_eq!(first.id(), first_id);
        runtime.finish_job(first);

        let second = runtime.next_job().await.unwrap();
        runtime.finish_job(second);

        assert_eq!(runtime.pending(), 0);
    }
}
