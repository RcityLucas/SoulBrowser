use std::collections::VecDeque;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use parking_lot::Mutex;

use crate::config::HotCfg;
use crate::model::{EventEnvelope, Filter};

const ACTION_RING_MAX: usize = 256;

#[derive(Debug, Default)]
pub struct EventRing {
    capacity: usize,
    queue: Mutex<VecDeque<EventEnvelope>>,
}

impl EventRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queue: Mutex::new(VecDeque::with_capacity(capacity)),
        }
    }

    pub fn push(&self, event: EventEnvelope) {
        let mut guard = self.queue.lock();
        if self.capacity > 0 && guard.len() >= self.capacity {
            guard.pop_front();
        }
        guard.push_back(event);
    }

    pub fn len(&self) -> usize {
        self.queue.lock().len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn snapshot(&self) -> Vec<EventEnvelope> {
        self.queue.lock().iter().cloned().collect()
    }

    pub fn collect_tail(&self, limit: usize, filter: Option<&Filter>) -> Vec<EventEnvelope> {
        let guard = self.queue.lock();
        let mut out = Vec::new();
        for event in guard.iter().rev() {
            if filter.map(|f| f.matches(event)).unwrap_or(true) {
                out.push(event.clone());
                if out.len() == limit {
                    break;
                }
            }
        }
        out.reverse();
        out
    }

    pub fn collect_since(
        &self,
        ts: DateTime<Utc>,
        limit: usize,
        filter: Option<&Filter>,
    ) -> Vec<EventEnvelope> {
        let guard = self.queue.lock();
        let mut out = Vec::new();
        for event in guard.iter() {
            if event.ts_wall < ts {
                continue;
            }
            if filter.map(|f| f.matches(event)).unwrap_or(true) {
                out.push(event.clone());
                if out.len() == limit {
                    break;
                }
            }
        }
        out
    }
}

#[derive(Debug)]
pub struct HotRings {
    cfg: HotCfg,
    global: Arc<EventRing>,
    sessions: DashMap<String, Arc<EventRing>>,
    pages: DashMap<String, Arc<EventRing>>,
    tasks: DashMap<String, Arc<EventRing>>,
    actions: DashMap<String, Mutex<VecDeque<EventEnvelope>>>,
}

impl HotRings {
    pub fn new(cfg: HotCfg) -> Self {
        Self {
            global: Arc::new(EventRing::new(cfg.n_global)),
            sessions: DashMap::new(),
            pages: DashMap::new(),
            tasks: DashMap::new(),
            actions: DashMap::new(),
            cfg,
        }
    }

    pub fn rings(&self) -> Arc<EventRing> {
        Arc::clone(&self.global)
    }

    pub fn utilization(&self) -> f32 {
        let len = self.global.len() as f32;
        let cap = self.global.capacity() as f32;
        if cap <= 0.0 {
            0.0
        } else {
            (len / cap).min(1.0)
        }
    }

    pub fn write(&self, event: EventEnvelope) {
        self.global.push(event.clone());

        if let Some(session) = &event.scope.session {
            self.insert_into_ring(
                &self.sessions,
                session.0.clone(),
                event.clone(),
                self.cfg.n_session,
            );
        }
        if let Some(page) = &event.scope.page {
            self.insert_into_ring(&self.pages, page.0.clone(), event.clone(), self.cfg.n_page);
        }
        if let Some(task) = &event.scope.task {
            self.insert_into_ring(&self.tasks, task.0.clone(), event.clone(), self.cfg.n_task);
        }
        if let Some(action) = &event.scope.action {
            self.insert_into_action(action.0.clone(), event);
        }
    }

    fn insert_into_ring(
        &self,
        map: &DashMap<String, Arc<EventRing>>,
        key: String,
        event: EventEnvelope,
        capacity: usize,
    ) {
        let ring = map
            .entry(key)
            .or_insert_with(|| Arc::new(EventRing::new(capacity)))
            .clone();
        ring.push(event);
    }

    fn insert_into_action(&self, key: String, event: EventEnvelope) {
        let entry = self
            .actions
            .entry(key)
            .or_insert_with(|| Mutex::new(VecDeque::with_capacity(ACTION_RING_MAX)));
        let mut guard = entry.lock();
        if guard.len() >= ACTION_RING_MAX {
            guard.pop_front();
        }
        guard.push_back(event);
    }

    pub fn tail(&self, limit: usize, filter: Option<&Filter>) -> Vec<EventEnvelope> {
        self.global.collect_tail(limit, filter)
    }

    pub fn since(
        &self,
        ts: DateTime<Utc>,
        limit: usize,
        filter: Option<&Filter>,
    ) -> Vec<EventEnvelope> {
        self.global.collect_since(ts, limit, filter)
    }

    pub fn by_action(&self, action: &str) -> Vec<EventEnvelope> {
        self.actions
            .get(action)
            .map(|ring| ring.lock().iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn snapshot(&self) -> Vec<EventEnvelope> {
        self.global.snapshot()
    }
}
