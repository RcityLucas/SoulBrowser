use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use soulbrowser_core_types::TaskId;
use tokio::{runtime::Handle, sync::broadcast};
use tracing::warn;
use uuid::Uuid;

use crate::judge::JudgeVerdict;
use crate::metrics::record_watchdog_event;
use crate::self_heal::SelfHealEvent;
use crate::watchdogs::{analyze_observation, WatchdogEvent};

const DEFAULT_LOG_CAPACITY: usize = 200;
pub const DEFAULT_TASK_LOG_PAGE_LIMIT: usize = 100;
pub const MAX_TASK_LOG_PAGE_SIZE: usize = 500;
const MAX_RECENT_EVIDENCE: usize = 8;
const MAX_OBSERVATION_HISTORY: usize = 50;
const MAX_AGENT_HISTORY: usize = 40;
const MAX_WATCHDOG_EVENTS: usize = 40;
const MAX_SELF_HEAL_EVENTS: usize = 40;
const MAX_ALERTS: usize = 20;
const TASK_STREAM_HISTORY_LIMIT: usize = 256;
static ALERT_WEBHOOK_URL: Lazy<Option<String>> =
    Lazy::new(|| match std::env::var("SOULBROWSER_ALERT_WEBHOOK") {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => None,
    });
static ALERT_WEBHOOK_CLIENT: Lazy<Client> = Lazy::new(Client::new);

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Success,
    Failed,
}

impl fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            ExecutionStatus::Pending => "pending",
            ExecutionStatus::Running => "running",
            ExecutionStatus::Success => "success",
            ExecutionStatus::Failed => "failed",
        };
        f.write_str(label)
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct TaskLogEntry {
    pub cursor: u64,
    pub timestamp: DateTime<Utc>,
    pub level: TaskLogLevel,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentHistoryStatus {
    Success,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentHistoryEntry {
    pub timestamp: DateTime<Utc>,
    pub step_index: usize,
    pub step_id: String,
    pub title: String,
    pub status: AgentHistoryStatus,
    pub attempts: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obstruction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TaskStatusSnapshot {
    pub task_id: String,
    pub title: String,
    pub status: ExecutionStatus,
    pub total_steps: usize,
    pub current_step: Option<usize>,
    pub current_step_title: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub last_updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_overlays: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_evidence: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observation_history: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_snapshot: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<TaskAnnotation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_history: Vec<AgentHistoryEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watchdog_events: Vec<WatchdogEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_verdict: Option<TaskJudgeVerdict>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub self_heal_events: Vec<SelfHealEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alerts: Vec<TaskAlert>,
}

struct TaskStatusRecord {
    task_id: String,
    title: String,
    status: ExecutionStatus,
    total_steps: usize,
    current_step: Option<usize>,
    current_step_title: Option<String>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    logs: VecDeque<TaskLogEntry>,
    last_updated_at: DateTime<Utc>,
    log_capacity: usize,
    next_log_cursor: u64,
    plan_overlays: Option<Value>,
    recent_evidence: Vec<Value>,
    observation_history: Vec<Value>,
    context_snapshot: Option<Value>,
    annotations: Vec<TaskAnnotation>,
    agent_history: Vec<AgentHistoryEntry>,
    watchdog_events: Vec<WatchdogEvent>,
    judge_verdict: Option<TaskJudgeVerdict>,
    self_heal_events: Vec<SelfHealEvent>,
    alerts: Vec<TaskAlert>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskAlert {
    pub timestamp: DateTime<Utc>,
    pub severity: String,
    pub message: String,
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskJudgeVerdict {
    pub verdict: JudgeVerdict,
    pub recorded_at: DateTime<Utc>,
}

impl TaskStatusRecord {
    fn new(task_id: String, title: String, total_steps: usize, log_capacity: usize) -> Self {
        Self {
            task_id,
            title,
            status: ExecutionStatus::Pending,
            total_steps,
            current_step: None,
            current_step_title: None,
            started_at: None,
            finished_at: None,
            last_error: None,
            logs: VecDeque::new(),
            last_updated_at: Utc::now(),
            log_capacity: log_capacity.max(1),
            next_log_cursor: 0,
            plan_overlays: None,
            recent_evidence: Vec::new(),
            observation_history: Vec::new(),
            context_snapshot: None,
            annotations: Vec::new(),
            agent_history: Vec::new(),
            watchdog_events: Vec::new(),
            judge_verdict: None,
            self_heal_events: Vec::new(),
            alerts: Vec::new(),
        }
    }

    fn touch(&mut self) {
        self.last_updated_at = Utc::now();
    }

    fn snapshot(&self) -> TaskStatusSnapshot {
        TaskStatusSnapshot {
            task_id: self.task_id.clone(),
            title: self.title.clone(),
            status: self.status,
            total_steps: self.total_steps,
            current_step: self.current_step,
            current_step_title: self.current_step_title.clone(),
            started_at: self.started_at,
            finished_at: self.finished_at,
            last_error: self.last_error.clone(),
            last_updated_at: self.last_updated_at,
            plan_overlays: self.plan_overlays.clone(),
            recent_evidence: self.recent_evidence.clone(),
            observation_history: self.observation_history.clone(),
            context_snapshot: self.context_snapshot.clone(),
            annotations: self.annotations.clone(),
            agent_history: self.agent_history.clone(),
            watchdog_events: self.watchdog_events.clone(),
            judge_verdict: self.judge_verdict.clone(),
            self_heal_events: self.self_heal_events.clone(),
            alerts: self.alerts.clone(),
        }
    }

    fn push_log(&mut self, level: TaskLogLevel, message: impl Into<String>) -> TaskLogEntry {
        let entry = TaskLogEntry {
            cursor: self.next_log_cursor,
            timestamp: Utc::now(),
            level,
            message: message.into(),
        };
        self.next_log_cursor = self.next_log_cursor.wrapping_add(1);
        if self.logs.len() == self.log_capacity {
            self.logs.pop_front();
        }
        self.logs.push_back(entry.clone());
        entry
    }
}

pub struct TaskStatusRegistry {
    records: DashMap<String, Mutex<TaskStatusRecord>>,
    log_capacity: usize,
    streams: DashMap<String, Arc<TaskStreamChannel>>,
}

impl TaskStatusRegistry {
    pub fn new(log_capacity: usize) -> Self {
        Self {
            records: DashMap::new(),
            log_capacity: log_capacity.max(1),
            streams: DashMap::new(),
        }
    }

    pub fn register(
        self: &Arc<Self>,
        task_id: TaskId,
        title: String,
        total_steps: usize,
    ) -> TaskStatusHandle {
        let id = task_id.0.clone();
        let record = TaskStatusRecord::new(id.clone(), title, total_steps, self.log_capacity);
        self.records.insert(id.clone(), Mutex::new(record));
        self.ensure_stream(&id);
        self.emit_status(&id);
        TaskStatusHandle {
            registry: Arc::clone(self),
            task_id,
        }
    }

    pub fn snapshot(&self, task_id: &str) -> Option<TaskStatusSnapshot> {
        let entry = self.records.get(task_id)?;
        let record = entry.value().lock();
        Some(record.snapshot())
    }

    pub fn logs_since(
        &self,
        task_id: &str,
        since: Option<DateTime<Utc>>,
        cursor: Option<u64>,
        limit: Option<usize>,
    ) -> Option<(Vec<TaskLogEntry>, Option<u64>)> {
        let entry = self.records.get(task_id)?;
        let record = entry.value().lock();
        let limit = limit
            .unwrap_or(DEFAULT_TASK_LOG_PAGE_LIMIT)
            .clamp(1, MAX_TASK_LOG_PAGE_SIZE);
        let mut items = Vec::new();
        let mut has_more = false;
        for log in record.logs.iter() {
            if let Some(cutoff) = since {
                if log.timestamp <= cutoff {
                    continue;
                }
            }
            if let Some(cursor_cutoff) = cursor {
                if log.cursor <= cursor_cutoff {
                    continue;
                }
            }
            if items.len() == limit {
                has_more = true;
                break;
            }
            items.push(log.clone());
        }
        let next_cursor = if has_more {
            items.last().map(|log| log.cursor)
        } else {
            None
        };
        Some((items, next_cursor))
    }

    pub fn annotations(&self, task_id: &str) -> Option<Vec<TaskAnnotation>> {
        let entry = self.records.get(task_id)?;
        let record = entry.value().lock();
        Some(record.annotations.clone())
    }

    pub fn observation_history(&self, task_id: &str, limit: usize) -> Option<Vec<Value>> {
        if limit == 0 {
            return Some(Vec::new());
        }
        let entry = self.records.get(task_id)?;
        let record = entry.value().lock();
        let history_len = record.observation_history.len();
        if history_len <= limit {
            Some(record.observation_history.clone())
        } else {
            Some(record.observation_history[history_len - limit..].to_vec())
        }
    }

    pub fn latest_observation(&self, task_id: &str) -> Option<Value> {
        let entry = self.records.get(task_id)?;
        let record = entry.value().lock();
        record.observation_history.last().cloned()
    }

    pub fn add_annotation(&self, task_id: &str, annotation: TaskAnnotation) -> bool {
        if let Some(entry) = self.records.get(task_id) {
            let mut record = entry.value().lock();
            record.annotations.push(annotation.clone());
            record.touch();
            drop(record);
            self.emit_status(task_id);
            self.emit_annotation(task_id, annotation);
            true
        } else {
            false
        }
    }

    pub fn all_snapshots(&self) -> Vec<TaskStatusSnapshot> {
        self.records
            .iter()
            .map(|entry| entry.value().lock().snapshot())
            .collect()
    }

    pub fn subscribe(&self, task_id: &str) -> Option<broadcast::Receiver<TaskStreamEnvelope>> {
        let channel = self.stream_channel(task_id)?;
        Some(channel.sender.subscribe())
    }

    pub fn mark_cancelled(&self, task_id: &str, reason: &str) {
        if let Some(entry) = self.records.get(task_id) {
            let mut record = entry.value().lock();
            record.status = ExecutionStatus::Failed;
            record.last_error = Some(reason.to_string());
            record.finished_at = Some(Utc::now());
            record.touch();
            let log_entry = record.push_log(TaskLogLevel::Warn, reason);
            drop(record);
            self.emit_status(task_id);
            self.push_stream_event(task_id, TaskStreamEvent::log(log_entry));
        }
    }

    fn ensure_stream(&self, task_id: &str) -> Arc<TaskStreamChannel> {
        if let Some(entry) = self.streams.get(task_id) {
            entry.value().clone()
        } else {
            let (tx, _) = broadcast::channel(64);
            let channel = Arc::new(TaskStreamChannel {
                sender: tx,
                history: Mutex::new(TaskStreamHistory::new()),
            });
            self.streams.insert(task_id.to_string(), channel.clone());
            channel
        }
    }

    fn stream_channel(&self, task_id: &str) -> Option<Arc<TaskStreamChannel>> {
        if let Some(entry) = self.streams.get(task_id) {
            Some(entry.value().clone())
        } else if self.records.contains_key(task_id) {
            Some(self.ensure_stream(task_id))
        } else {
            None
        }
    }

    pub fn stream_history_since(
        &self,
        task_id: &str,
        last_event_id: Option<u64>,
    ) -> Option<Vec<TaskStreamEnvelope>> {
        let channel = self.stream_channel(task_id)?;
        let history = channel.history.lock();
        Some(history.since(last_event_id))
    }

    fn emit_status(&self, task_id: &str) {
        let snapshot = match self.snapshot(task_id) {
            Some(snapshot) => snapshot,
            None => return,
        };
        self.push_stream_event(task_id, TaskStreamEvent::status(snapshot));
    }

    fn emit_log(&self, task_id: &str, entry: TaskLogEntry) {
        self.push_stream_events(task_id, vec![TaskStreamEvent::log(entry)]);
    }

    fn emit_agent_history(&self, task_id: &str, entry: AgentHistoryEntry) {
        self.push_stream_events(task_id, vec![TaskStreamEvent::agent_history(entry)]);
    }

    fn emit_context(&self, task_id: &str, snapshot: Value) {
        self.push_stream_events(task_id, vec![TaskStreamEvent::context(snapshot)]);
    }

    fn emit_observations(&self, task_id: &str, artifacts: &[Value]) {
        if artifacts.is_empty() {
            return;
        }
        let mut events = Vec::with_capacity(artifacts.len());
        for artifact in artifacts {
            let payload = observation_payload_from_artifact(task_id, artifact.clone());
            self.run_watchdogs(task_id, &payload);
            events.push(TaskStreamEvent::observation(payload));
        }
        self.push_stream_events(task_id, events);
    }

    fn run_watchdogs(&self, task_id: &str, payload: &ObservationPayload) {
        let findings = analyze_observation(payload);
        if findings.is_empty() {
            return;
        }
        for finding in findings {
            if let Some(annotation) = finding.annotation.clone() {
                let inserted = self.add_annotation(task_id, annotation);
                if !inserted {
                    break;
                }
            }
            self.add_watchdog_event(task_id, finding.event.clone());
            record_watchdog_event(&finding.event.kind);
            let severity = finding.event.severity.to_ascii_lowercase();
            if severity == "warn" || severity == "critical" {
                let alert = TaskAlert {
                    timestamp: finding.event.recorded_at,
                    severity: severity.clone(),
                    message: finding.event.note.clone(),
                    kind: Some(finding.event.kind.clone()),
                };
                self.add_alert(task_id, alert);
            }
        }
    }

    fn emit_plan_overlays(&self, task_id: &str, overlays: &Value) {
        self.emit_overlays(task_id, OverlaySource::Plan, overlays);
    }

    fn emit_execution_overlays(&self, task_id: &str, overlays: &Value) {
        self.emit_overlays(task_id, OverlaySource::Execution, overlays);
    }

    fn emit_overlays(&self, task_id: &str, source: OverlaySource, overlays: &Value) {
        let Some(items) = overlays.as_array() else {
            return;
        };
        if items.is_empty() {
            return;
        }
        let mut events = Vec::with_capacity(items.len());
        for item in items {
            let payload = OverlayPayload {
                task_id: task_id.to_string(),
                source,
                recorded_at: extract_recorded_at(item),
                data: item.clone(),
            };
            events.push(TaskStreamEvent::overlay(payload));
        }
        self.push_stream_events(task_id, events);
    }

    fn emit_annotation(&self, task_id: &str, annotation: TaskAnnotation) {
        self.push_stream_events(task_id, vec![TaskStreamEvent::annotation(annotation)]);
    }

    fn add_alert(&self, task_id: &str, alert: TaskAlert) {
        if let Some(entry) = self.records.get(task_id) {
            let mut record = entry.lock();
            if record.alerts.len() == MAX_ALERTS {
                record.alerts.remove(0);
            }
            record.alerts.push(alert.clone());
        }
        self.emit_alert(task_id, alert);
    }

    fn add_watchdog_event(&self, task_id: &str, event: WatchdogEvent) {
        if let Some(entry) = self.records.get(task_id) {
            let mut record = entry.lock();
            if record.watchdog_events.len() == MAX_WATCHDOG_EVENTS {
                record.watchdog_events.remove(0);
            }
            record.watchdog_events.push(event.clone());
        }
        self.emit_watchdog_event(task_id, event);
    }

    fn emit_watchdog_event(&self, task_id: &str, event: WatchdogEvent) {
        self.push_stream_events(task_id, vec![TaskStreamEvent::watchdog(event)]);
    }

    fn emit_judge_event(&self, task_id: &str, verdict: TaskJudgeVerdict) {
        self.push_stream_events(task_id, vec![TaskStreamEvent::judge(verdict)]);
    }

    fn emit_self_heal_event(&self, task_id: &str, event: SelfHealEvent) {
        self.push_stream_events(task_id, vec![TaskStreamEvent::self_heal(event)]);
    }

    fn emit_alert(&self, task_id: &str, alert: TaskAlert) {
        self.push_stream_events(task_id, vec![TaskStreamEvent::alert(alert.clone())]);
        send_alert_webhook(task_id, &alert);
    }

    fn push_stream_event(&self, task_id: &str, event: TaskStreamEvent) {
        self.push_stream_events(task_id, vec![event]);
    }

    fn push_stream_events(&self, task_id: &str, events: Vec<TaskStreamEvent>) {
        if events.is_empty() {
            return;
        }
        let Some(channel) = self.stream_channel(task_id) else {
            return;
        };
        let envelopes = {
            let mut history = channel.history.lock();
            events
                .into_iter()
                .map(|event| history.record(event))
                .collect::<Vec<_>>()
        };
        for envelope in envelopes {
            let _ = channel.sender.send(envelope);
        }
    }
}

#[derive(Clone)]
pub struct TaskStatusHandle {
    registry: Arc<TaskStatusRegistry>,
    task_id: TaskId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskAnnotation {
    pub id: String,
    pub step_id: Option<String>,
    pub dispatch_label: Option<String>,
    pub note: String,
    pub bbox: Option<Value>,
    pub author: Option<String>,
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl TaskAnnotation {
    pub fn new(
        step_id: Option<String>,
        dispatch_label: Option<String>,
        note: String,
        bbox: Option<Value>,
        author: Option<String>,
        severity: Option<String>,
        kind: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            step_id,
            dispatch_label,
            note,
            bbox,
            author,
            severity,
            kind,
            created_at: Utc::now(),
        }
    }
}

impl fmt::Debug for TaskStatusHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskStatusHandle")
            .field("task_id", &self.task_id)
            .finish()
    }
}

impl TaskStatusHandle {
    pub fn update_plan(&self, title: String, total_steps: usize) {
        if self.with_record(|record| {
            record.title = title;
            record.total_steps = total_steps;
            record.current_step = None;
            record.current_step_title = None;
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
        }
        self.log(TaskLogLevel::Info, "Plan updated after replanning");
    }

    pub fn set_plan_overlays(&self, overlays: Value) {
        let overlays_for_emit = overlays.clone();
        if self.with_record(|record| {
            record.plan_overlays = Some(overlays);
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
        }
        self.registry
            .emit_plan_overlays(&self.task_id.0, &overlays_for_emit);
    }

    pub fn set_context_snapshot(&self, context: Option<Value>) {
        if self.with_record(|record| {
            record.context_snapshot = context.clone();
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
            if let Some(value) = context {
                self.registry.emit_context(&self.task_id.0, value);
            }
        }
    }

    pub fn push_evidence(&self, artifacts: &[Value]) {
        if artifacts.is_empty() {
            return;
        }
        if self.with_record(|record| {
            record.recent_evidence.extend(artifacts.iter().cloned());
            if record.recent_evidence.len() > MAX_RECENT_EVIDENCE {
                let overflow = record.recent_evidence.len() - MAX_RECENT_EVIDENCE;
                record.recent_evidence.drain(0..overflow);
            }
            record.observation_history.extend(artifacts.iter().cloned());
            if record.observation_history.len() > MAX_OBSERVATION_HISTORY {
                let overflow = record.observation_history.len() - MAX_OBSERVATION_HISTORY;
                record.observation_history.drain(0..overflow);
            }
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
            self.registry.emit_observations(&self.task_id.0, artifacts);
        }
    }

    pub fn push_alert(&self, alert: TaskAlert) {
        if self.with_record(|record| {
            if record.alerts.len() == MAX_ALERTS {
                record.alerts.remove(0);
            }
            record.alerts.push(alert.clone());
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
        }
        self.registry.emit_alert(&self.task_id.0, alert);
    }

    pub fn push_agent_history(&self, entry: AgentHistoryEntry) {
        if self.with_record(|record| {
            record.agent_history.push(entry.clone());
            if record.agent_history.len() > MAX_AGENT_HISTORY {
                let overflow = record.agent_history.len() - MAX_AGENT_HISTORY;
                record.agent_history.drain(0..overflow);
            }
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
            self.registry.emit_agent_history(&self.task_id.0, entry);
        }
    }

    pub fn push_execution_overlays(&self, overlays: Value) {
        if overlays.as_array().map_or(true, |items| items.is_empty()) {
            return;
        }
        self.registry
            .emit_execution_overlays(&self.task_id.0, &overlays);
    }

    pub fn mark_running(&self) {
        if self.with_record(|record| {
            record.status = ExecutionStatus::Running;
            if record.started_at.is_none() {
                record.started_at = Some(Utc::now());
            }
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
        }
        self.log(TaskLogLevel::Info, "Execution started");
    }

    pub fn mark_success(&self) {
        if self.with_record(|record| {
            record.status = ExecutionStatus::Success;
            record.finished_at = Some(Utc::now());
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
        }
        self.log(TaskLogLevel::Info, "Execution completed successfully");
    }

    pub fn mark_failure(&self, error: Option<String>) {
        if self.with_record(|record| {
            record.status = ExecutionStatus::Failed;
            record.finished_at = Some(Utc::now());
            record.last_error = error.clone();
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
        }
        if let Some(err) = error {
            self.log(TaskLogLevel::Error, format!("Execution failed: {err}"));
        } else {
            self.log(TaskLogLevel::Error, "Execution failed");
        }
    }

    pub fn set_judge_verdict(&self, verdict: JudgeVerdict) {
        let payload = TaskJudgeVerdict {
            verdict,
            recorded_at: Utc::now(),
        };
        if self.with_record(|record| {
            record.judge_verdict = Some(payload.clone());
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
        }
        self.registry.emit_judge_event(&self.task_id.0, payload);
    }

    pub fn push_self_heal_event(&self, event: SelfHealEvent) {
        if self.with_record(|record| {
            if record.self_heal_events.len() == MAX_SELF_HEAL_EVENTS {
                record.self_heal_events.remove(0);
            }
            record.self_heal_events.push(event.clone());
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
        }
        self.registry.emit_self_heal_event(&self.task_id.0, event);
    }

    pub fn snapshot(&self) -> Option<TaskStatusSnapshot> {
        self.registry.snapshot(&self.task_id.0)
    }

    pub fn step_started(&self, index: usize, title: &str) {
        if self.with_record(|record| {
            record.current_step = Some(index);
            record.current_step_title = Some(title.to_string());
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
        }
        self.log(
            TaskLogLevel::Info,
            format!("Step {} started: {}", index + 1, title),
        );
    }

    pub fn step_completed(&self, index: usize, title: &str) {
        self.log(
            TaskLogLevel::Info,
            format!("Step {} completed: {}", index + 1, title),
        );
        if self.with_record(|record| {
            if record.current_step == Some(index) {
                record.current_step_title = None;
            }
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
        }
    }

    pub fn step_failed(&self, index: usize, title: &str, error: &str) {
        self.log(
            TaskLogLevel::Error,
            format!("Step {} failed ({}): {}", index + 1, title, error),
        );
        if self.with_record(|record| {
            record.current_step = Some(index);
            record.current_step_title = Some(title.to_string());
            record.last_error = Some(error.to_string());
            record.touch();
        }) {
            self.registry.emit_status(&self.task_id.0);
        }
    }

    pub fn log(&self, level: TaskLogLevel, message: impl Into<String>) {
        let msg = message.into();
        let mut emitted: Option<TaskLogEntry> = None;
        if self.with_record(|record| {
            let entry = record.push_log(level, msg.clone());
            record.touch();
            emitted = Some(entry);
        }) {
            if let Some(entry) = emitted {
                self.registry.emit_log(&self.task_id.0, entry);
            }
        }
    }

    fn with_record<F>(&self, update: F) -> bool
    where
        F: FnOnce(&mut TaskStatusRecord),
    {
        if let Some(entry) = self.registry.records.get(&self.task_id.0) {
            let mut record = entry.value().lock();
            update(&mut record);
            true
        } else {
            false
        }
    }

    pub fn add_annotation(&self, annotation: TaskAnnotation) {
        let _ = self.registry.add_annotation(&self.task_id.0, annotation);
    }
}

impl Default for TaskStatusRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_LOG_CAPACITY)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TaskStreamEvent {
    Status {
        status: TaskStatusSnapshot,
    },
    Log {
        log: TaskLogEntry,
    },
    Context {
        context: Value,
    },
    Observation {
        #[serde(flatten)]
        observation: ObservationPayload,
    },
    Overlay {
        overlay: OverlayPayload,
    },
    Annotation {
        annotation: TaskAnnotation,
    },
    AgentHistory {
        entry: AgentHistoryEntry,
    },
    Watchdog {
        watchdog: WatchdogEvent,
    },
    Judge {
        verdict: TaskJudgeVerdict,
    },
    SelfHeal {
        self_heal: SelfHealEvent,
    },
    Alert {
        alert: TaskAlert,
    },
}

impl TaskStreamEvent {
    #[allow(dead_code)]
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            TaskStreamEvent::Status { .. } => "status",
            TaskStreamEvent::Log { .. } => "log",
            TaskStreamEvent::Context { .. } => "context",
            TaskStreamEvent::Observation { .. } => "observation",
            TaskStreamEvent::Overlay { .. } => "overlay",
            TaskStreamEvent::Annotation { .. } => "annotation",
            TaskStreamEvent::AgentHistory { .. } => "agent_history",
            TaskStreamEvent::Watchdog { .. } => "watchdog",
            TaskStreamEvent::Judge { .. } => "judge",
            TaskStreamEvent::SelfHeal { .. } => "self_heal",
            TaskStreamEvent::Alert { .. } => "alert",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TaskStreamEnvelope {
    pub id: u64,
    pub event: TaskStreamEvent,
}

struct TaskStreamHistory {
    next_id: u64,
    events: VecDeque<TaskStreamEnvelope>,
}

impl TaskStreamHistory {
    fn new() -> Self {
        Self {
            next_id: 0,
            events: VecDeque::new(),
        }
    }

    fn record(&mut self, event: TaskStreamEvent) -> TaskStreamEnvelope {
        let envelope = TaskStreamEnvelope {
            id: self.next_id,
            event,
        };
        self.next_id = self.next_id.wrapping_add(1);
        self.events.push_back(envelope.clone());
        if self.events.len() > TASK_STREAM_HISTORY_LIMIT {
            self.events.pop_front();
        }
        envelope
    }

    fn since(&self, cursor: Option<u64>) -> Vec<TaskStreamEnvelope> {
        self.events
            .iter()
            .filter(|env| cursor.map(|id| env.id > id).unwrap_or(true))
            .cloned()
            .collect()
    }
}

struct TaskStreamChannel {
    sender: broadcast::Sender<TaskStreamEnvelope>,
    history: Mutex<TaskStreamHistory>,
}

impl TaskStreamEvent {
    pub fn status(snapshot: TaskStatusSnapshot) -> Self {
        TaskStreamEvent::Status { status: snapshot }
    }

    pub fn log(entry: TaskLogEntry) -> Self {
        TaskStreamEvent::Log { log: entry }
    }

    pub fn context(value: Value) -> Self {
        TaskStreamEvent::Context { context: value }
    }

    pub fn observation(observation: ObservationPayload) -> Self {
        TaskStreamEvent::Observation { observation }
    }

    pub fn overlay(overlay: OverlayPayload) -> Self {
        TaskStreamEvent::Overlay { overlay }
    }

    pub fn annotation(annotation: TaskAnnotation) -> Self {
        TaskStreamEvent::Annotation { annotation }
    }

    pub fn agent_history(entry: AgentHistoryEntry) -> Self {
        TaskStreamEvent::AgentHistory { entry }
    }

    pub fn watchdog(event: WatchdogEvent) -> Self {
        TaskStreamEvent::Watchdog { watchdog: event }
    }

    pub fn judge(verdict: TaskJudgeVerdict) -> Self {
        TaskStreamEvent::Judge { verdict }
    }

    pub fn self_heal(event: SelfHealEvent) -> Self {
        TaskStreamEvent::SelfHeal { self_heal: event }
    }

    pub fn alert(alert: TaskAlert) -> Self {
        TaskStreamEvent::Alert { alert }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ObservationPayload {
    #[serde(default)]
    pub observation_type: String,
    #[serde(default)]
    pub task_id: String,
    #[serde(default)]
    pub step_id: Option<String>,
    #[serde(default)]
    pub dispatch_label: Option<String>,
    #[serde(default)]
    pub dispatch_index: Option<usize>,
    #[serde(default)]
    pub screenshot_path: Option<String>,
    #[serde(default)]
    pub bbox: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    pub recorded_at: DateTime<Utc>,
    pub artifact: Value,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlaySource {
    Plan,
    Execution,
}

#[derive(Clone, Debug, Serialize)]
pub struct OverlayPayload {
    pub task_id: String,
    pub source: OverlaySource,
    pub recorded_at: DateTime<Utc>,
    pub data: Value,
}

pub fn observation_payload_from_artifact(task_id: &str, artifact: Value) -> ObservationPayload {
    let observation_type = artifact
        .get("content_type")
        .and_then(|v| v.as_str())
        .map(|ct| {
            if ct.starts_with("image/") {
                "image"
            } else {
                "artifact"
            }
        })
        .unwrap_or("artifact")
        .to_string();
    let content_type = artifact
        .get("content_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let screenshot_path = artifact
        .get("path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let bbox = artifact.get("bbox").cloned();
    let step_id = artifact
        .get("step_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let dispatch_label = artifact
        .get("dispatch_label")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let dispatch_index = artifact
        .get("dispatch_index")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let recorded_at = extract_recorded_at(&artifact);

    ObservationPayload {
        observation_type,
        task_id: task_id.to_string(),
        step_id,
        dispatch_label,
        dispatch_index,
        screenshot_path,
        bbox,
        content_type,
        recorded_at,
        artifact,
    }
}

fn extract_recorded_at(value: &Value) -> DateTime<Utc> {
    value
        .get("recorded_at")
        .and_then(|v| v.as_str())
        .and_then(|datetime| {
            DateTime::parse_from_rfc3339(datetime)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        })
        .unwrap_or_else(Utc::now)
}

fn send_alert_webhook(task_id: &str, alert: &TaskAlert) {
    let Some(url) = ALERT_WEBHOOK_URL.as_ref() else {
        return;
    };
    if Handle::try_current().is_err() {
        return;
    }
    let payload = json!({
        "task_id": task_id,
        "severity": alert.severity,
        "message": alert.message,
        "kind": alert.kind,
        "timestamp": alert.timestamp.to_rfc3339(),
    });
    let url = url.clone();
    tokio::spawn(async move {
        let client = ALERT_WEBHOOK_CLIENT.clone();
        if let Err(err) = client.post(&url).json(&payload).send().await {
            warn!(target: "alerts", ?err, "failed to deliver alert webhook");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn registry_tracks_status_snapshots() {
        let registry = Arc::new(TaskStatusRegistry::new(8));
        let handle = registry.register(TaskId::new(), "Test".into(), 2);
        handle.mark_running();
        handle.step_started(0, "Open page");
        handle.step_failed(0, "Open page", "Timeout");
        handle.mark_failure(Some("Timeout".into()));

        let snapshots = registry.all_snapshots();
        assert_eq!(snapshots.len(), 1);
        let snapshot = &snapshots[0];
        assert_eq!(snapshot.status, ExecutionStatus::Failed);
        assert_eq!(snapshot.current_step, Some(0));

        let (logs, next) = registry
            .logs_since(&handle.task_id.0, None, None, None)
            .unwrap();
        assert!(!logs.is_empty());
        assert!(next.is_none());
    }

    #[tokio::test]
    async fn registry_streams_status_and_logs() {
        let registry = Arc::new(TaskStatusRegistry::new(8));
        let handle = registry.register(TaskId::new(), "Streaming".into(), 1);
        let mut stream = registry.subscribe(&handle.task_id.0).expect("receiver");

        handle.mark_running();
        let status_event = timeout(Duration::from_millis(100), stream.recv())
            .await
            .expect("status available")
            .expect("status event");
        match status_event.event {
            TaskStreamEvent::Status { status } => {
                assert_eq!(status.status, ExecutionStatus::Running);
            }
            other => panic!("expected status event, got {other:?}"),
        }

        handle.log(TaskLogLevel::Info, "Streaming test log");
        let mut found_stream_log = false;
        for _ in 0..4 {
            let event = timeout(Duration::from_millis(100), stream.recv())
                .await
                .expect("log available")
                .expect("log event");
            if let TaskStreamEvent::Log { log } = event.event {
                if log.message.contains("Streaming") {
                    assert_eq!(log.level, TaskLogLevel::Info);
                    found_stream_log = true;
                    break;
                }
            }
        }
        assert!(found_stream_log, "streaming log entry not observed");
    }
}
