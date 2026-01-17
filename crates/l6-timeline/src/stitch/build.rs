use super::fold::apply_fold;
use crate::model::{
    ActionNode, ArtifactsRefs, DecisionNode, EventEnvelope, EvidenceNode, ExportStats, GateDigest,
    JsonlLine, RecordLine, ReplayBundle, ReplayEvent, ReplayLine, TimelineFrame, TimelineLine,
    View,
};
use crate::policy::TimelinePolicyView;
use chrono::Utc;
use serde_json::Value as JsonValue;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug)]
pub struct BuildOutput {
    pub lines: Vec<JsonlLine>,
    pub stats: ExportStats,
}

pub fn build_lines(
    events: Vec<EventEnvelope>,
    view: &View,
    policy: &TimelinePolicyView,
) -> BuildOutput {
    let mut prepared = events;
    sort_and_dedup(&mut prepared);
    let folded = apply_fold(prepared, policy.fold_noise);

    let mut stats = ExportStats::default();
    let mut lines = Vec::new();

    match view {
        View::Records => {
            let records = build_records(&folded, policy.records_sample_rate);
            stats.total_lines = records.len();
            stats.total_actions = count_actions(&records);
            for record in records {
                lines.push(JsonlLine::Record(record));
            }
        }
        View::Timeline => {
            let timeline = build_timeline(&folded);
            stats.total_lines = timeline.len();
            stats.total_actions = timeline.len();
            for item in timeline {
                lines.push(JsonlLine::Timeline(item));
            }
        }
        View::Replay => {
            let replay = build_replay(&folded);
            stats.total_lines = replay.len();
            stats.total_actions = replay.len();
            for item in replay {
                lines.push(JsonlLine::Replay(item));
            }
        }
    }

    BuildOutput { lines, stats }
}

pub fn sort_and_dedup(events: &mut Vec<EventEnvelope>) {
    events.sort_by(|a, b| match a.ts_mono.cmp(&b.ts_mono) {
        Ordering::Equal => match a.seq.cmp(&b.seq) {
            Ordering::Equal => match a.action_id.cmp(&b.action_id) {
                Ordering::Equal => a.kind.cmp(&b.kind),
                other => other,
            },
            other => other,
        },
        other => other,
    });

    events.dedup_by(|a, b| {
        a.ts_mono == b.ts_mono && a.seq == b.seq && a.action_id == b.action_id && a.kind == b.kind
    });
}

fn build_records(events: &[EventEnvelope], sample_rate: f32) -> Vec<RecordLine> {
    if events.is_empty() {
        return Vec::new();
    }

    let rate = sample_rate.clamp(0.0, 1.0);
    let mut records = Vec::with_capacity(events.len());
    let mut skip_stride = if rate <= 0.0 {
        usize::MAX
    } else if (rate - 1.0).abs() < f32::EPSILON {
        1
    } else {
        (1.0 / rate).round() as usize
    };
    if skip_stride == 0 {
        skip_stride = 1;
    }

    for (idx, ev) in events.iter().enumerate() {
        if skip_stride != 1 && idx % skip_stride != 0 {
            continue;
        }
        records.push(RecordLine {
            stage: classify_stage(&ev.kind),
            action_id: ev.action_id.clone(),
            kind: ev.kind.clone(),
            ts_mono: ev.ts_mono,
            payload: ev.payload.clone(),
        });
    }

    records
}

fn build_timeline(events: &[EventEnvelope]) -> Vec<TimelineLine> {
    let mut grouped: BTreeMap<&str, Vec<&EventEnvelope>> = BTreeMap::new();
    for event in events {
        let key = event.action_id.as_str();
        grouped.entry(key).or_default().push(event);
    }

    let mut output = Vec::with_capacity(grouped.len());
    for (action_id, evs) in grouped {
        let frame = build_frame(action_id, &evs);
        output.push(TimelineLine {
            action_id: action_id.to_string(),
            frame,
        });
    }
    output
}

fn build_replay(events: &[EventEnvelope]) -> Vec<ReplayLine> {
    let mut grouped: BTreeMap<&str, Vec<&EventEnvelope>> = BTreeMap::new();
    for event in events {
        let key = event.action_id.as_str();
        grouped.entry(key).or_default().push(event);
    }

    let mut output = Vec::with_capacity(grouped.len());
    for (action_id, evs) in grouped {
        let mut bundle = ReplayBundle {
            action_id: action_id.to_string(),
            ..ReplayBundle::default()
        };
        bundle.timeline = build_replay_events(&evs);
        bundle.evidence = build_artifacts(&evs);
        bundle.summary = derive_summary(&evs);
        output.push(ReplayLine { bundle });
    }
    output
}

fn build_frame(_action_id: &str, events: &[&EventEnvelope]) -> TimelineFrame {
    let mut frame = TimelineFrame::default();
    frame.action.tool = infer_tool(events);
    frame.action.primitive = infer_primitive(events);

    let mut evidence_signals = Vec::new();
    let mut evidence_artifacts = ArtifactsRefs::default();
    let mut decision = DecisionNode::default();

    for ev in events {
        let stage = classify_stage(&ev.kind);
        match stage {
            "action" => update_action(&mut frame.action, ev),
            "evidence" => {
                evidence_signals.push(ev.payload.clone());
                merge_artifacts(&mut evidence_artifacts, &ev.payload);
            }
            "decision" => {
                update_decision(&mut decision, ev);
            }
            _ => {}
        }
    }

    if !evidence_signals.is_empty() {
        frame.evidence = Some(EvidenceNode {
            post_signals: JsonValue::Array(evidence_signals),
            artifacts: evidence_artifacts,
        });
    }

    if decision.gate.is_some() || decision.task_completed.is_some() || decision.insight.is_some() {
        frame.decision = Some(decision);
    }

    frame
}

fn build_replay_events(events: &[&EventEnvelope]) -> Vec<ReplayEvent> {
    if events.is_empty() {
        return Vec::new();
    }
    let base = events[0].ts_mono;
    events
        .iter()
        .map(|ev| ReplayEvent {
            delta_ms: ev.ts_mono - base,
            kind: ev.kind.clone(),
            digest: ev.payload.clone(),
        })
        .collect()
}

fn build_artifacts(events: &[&EventEnvelope]) -> ArtifactsRefs {
    let mut pix = HashSet::new();
    let mut structs = HashSet::new();
    for ev in events {
        if let Some(obj) = ev.payload.as_object() {
            if let Some(arr) = obj.get("pix_ids").and_then(|v| v.as_array()) {
                for item in arr {
                    if let Some(id) = item.as_str() {
                        pix.insert(id.to_string());
                    }
                }
            }
            if let Some(arr) = obj.get("struct_ids").and_then(|v| v.as_array()) {
                for item in arr {
                    if let Some(id) = item.as_str() {
                        structs.insert(id.to_string());
                    }
                }
            }
        }
    }
    ArtifactsRefs {
        pix: pix.into_iter().collect(),
        structs: structs.into_iter().collect(),
    }
}

fn derive_summary(events: &[&EventEnvelope]) -> Option<String> {
    events.iter().find_map(|ev| {
        ev.payload
            .as_object()
            .and_then(|obj| obj.get("summary"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })
}

fn classify_stage(kind: &str) -> &'static str {
    if kind.starts_with("ACT_") {
        "action"
    } else if kind.contains("VIS")
        || kind.contains("POST")
        || kind.contains("OBS")
        || kind.contains("NR_")
    {
        "evidence"
    } else if kind.contains("GATE") || kind.contains("TASK_COMPLETED") || kind.contains("INSIGHT") {
        "decision"
    } else {
        "event"
    }
}

fn update_action(node: &mut ActionNode, event: &EventEnvelope) {
    if let Some(obj) = event.payload.as_object() {
        if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
            node.tool = tool.to_string();
        }
        if let Some(primitive) = obj.get("primitive").and_then(|v| v.as_str()) {
            node.primitive = primitive.to_string();
        }
        if node.wait_tier.is_none() {
            node.wait_tier = obj
                .get("wait_tier")
                .and_then(|v| v.as_str().map(|s| s.to_string()));
        }
        if node.latency_ms.is_none() {
            node.latency_ms = obj.get("latency_ms").and_then(|v| v.as_u64());
        }
        if node.precheck.is_none() && event.kind.contains("PRECHECK") {
            node.precheck = Some(event.payload.clone());
        }
        if node.ok.is_none() && event.kind.contains("FINISH") {
            node.ok = Some(true);
        }
        if event.kind.contains("FAIL") || event.kind.contains("ERROR") {
            node.ok = Some(false);
            node.error = obj
                .get("error")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| Some(event.kind.clone()));
        }
    }
}

fn merge_artifacts(target: &mut ArtifactsRefs, payload: &JsonValue) {
    if let Some(obj) = payload.as_object() {
        if let Some(pix_ids) = obj.get("pix_ids").and_then(|v| v.as_array()) {
            for item in pix_ids {
                if let Some(id) = item.as_str() {
                    if !target.pix.contains(&id.to_string()) {
                        target.pix.push(id.to_string());
                    }
                }
            }
        }
        if let Some(struct_ids) = obj.get("struct_ids").and_then(|v| v.as_array()) {
            for item in struct_ids {
                if let Some(id) = item.as_str() {
                    if !target.structs.contains(&id.to_string()) {
                        target.structs.push(id.to_string());
                    }
                }
            }
        }
    }
}

fn update_decision(decision: &mut DecisionNode, event: &EventEnvelope) {
    if let Some(obj) = event.payload.as_object() {
        if event.kind.contains("GATE") {
            let pass = obj.get("pass").and_then(|v| v.as_bool()).unwrap_or(false);
            let reasons = obj
                .get("reasons")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            decision.gate = Some(GateDigest { pass, reasons });
        }
        if event.kind.contains("TASK_COMPLETED") {
            decision.task_completed = Some(crate::model::TaskDigest {
                completed: true,
                latency_ms: obj.get("latency_ms").and_then(|v| v.as_u64()),
            });
        }
        if event.kind.contains("INSIGHT") {
            decision.insight = Some(crate::model::InsightDigest {
                summary: obj
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            });
        }
    }
}

fn infer_tool(events: &[&EventEnvelope]) -> String {
    events
        .iter()
        .find_map(|ev| {
            ev.payload
                .as_object()
                .and_then(|obj| obj.get("tool"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("unknown")
        .to_string()
}

fn infer_primitive(events: &[&EventEnvelope]) -> String {
    events
        .iter()
        .find_map(|ev| {
            ev.payload
                .as_object()
                .and_then(|obj| obj.get("primitive"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("unknown")
        .to_string()
}

fn count_actions(records: &[RecordLine]) -> usize {
    let mut unique = HashSet::new();
    for rec in records {
        unique.insert(rec.action_id.clone());
    }
    unique.len()
}

pub fn build_header(policy_snapshot: JsonValue) -> JsonlLine {
    JsonlLine::Header(crate::model::HeaderLine {
        export_version: "v1",
        generated_at: Utc::now(),
        policy_snapshot,
    })
}

pub fn build_footer(stats: &ExportStats) -> JsonlLine {
    JsonlLine::Footer(crate::model::FooterLine {
        total_actions: stats.total_actions,
        total_lines: stats.total_lines,
        truncated: stats.truncated,
    })
}
