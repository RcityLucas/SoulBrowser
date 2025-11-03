use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::{json, Value};

use crate::model::{DomAxDiff, DomAxSnapshot};
use crate::policy::{DiffPolicy, DiffPolicyFocus};

pub fn diff(base: &DomAxSnapshot, current: &DomAxSnapshot) -> DomAxDiff {
    diff_with_policy(base, current, None)
}

pub fn diff_with_policy(
    base: &DomAxSnapshot,
    current: &DomAxSnapshot,
    policy: Option<&DiffPolicy>,
) -> DomAxDiff {
    if let Some(policy) = policy {
        if let Some(debounce) = policy.debounce_ms {
            let threshold = Duration::from_millis(debounce);
            let elapsed = if current.captured_at >= base.captured_at {
                current.captured_at.duration_since(base.captured_at)
            } else {
                base.captured_at.duration_since(current.captured_at)
            };
            if elapsed < threshold {
                let mut diff = DomAxDiff::empty();
                diff.base = Some(base.id.clone());
                diff.current = Some(current.id.clone());
                if let Some(focus) = policy
                    .focus
                    .as_ref()
                    .and_then(DiffPolicyFocus::to_model_focus)
                {
                    diff.focus = Some(focus);
                }
                diff.changes.push(json!({
                    "kind": "debounced",
                    "elapsed_ms": elapsed.as_millis(),
                    "threshold_ms": debounce,
                }));
                return diff;
            }
        }
    }

    let mut diff = diff_internal(base, current);
    if let Some(policy) = policy {
        if let Some(focus) = policy.focus.as_ref() {
            if let Some(model_focus) = focus.to_model_focus() {
                diff.focus = Some(model_focus);
            }
            diff.changes.insert(
                0,
                json!({
                    "kind": "focus",
                    "focus": focus.to_json(),
                }),
            );
        }
        if let Some(max) = policy.max_changes {
            if diff.changes.len() > max {
                diff.changes.truncate(max);
            }
        }
    }
    diff
}

fn diff_internal(base: &DomAxSnapshot, current: &DomAxSnapshot) -> DomAxDiff {
    let dom_before = summarize_dom(&base.dom_raw);
    let dom_after = summarize_dom(&current.dom_raw);

    let ax_before = summarize_ax(&base.ax_raw);
    let ax_after = summarize_ax(&current.ax_raw);

    let mut changes = Vec::new();
    collect_dom_changes(&dom_before, &dom_after, &mut changes);
    collect_ax_changes(&ax_before, &ax_after, &mut changes);

    let mut diff = DomAxDiff::empty();
    diff.base = Some(base.id.clone());
    diff.current = Some(current.id.clone());
    diff.changes = changes;
    diff
}

#[derive(Default)]
struct DomSummary {
    node_count: usize,
    text_samples: BTreeSet<String>,
    attribute_keys: BTreeSet<String>,
}

#[derive(Default)]
struct AxSummary {
    node_count: usize,
    roles: BTreeSet<String>,
    actions: BTreeSet<String>,
}

fn summarize_dom(snapshot: &Value) -> DomSummary {
    let mut summary = DomSummary::default();
    let strings = snapshot
        .get("strings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if let Some(documents) = snapshot.get("documents").and_then(Value::as_array) {
        for document in documents {
            if let Some(nodes) = document.get("nodes").and_then(Value::as_object) {
                if let Some(node_names) = nodes.get("nodeName").and_then(Value::as_array) {
                    summary.node_count += node_names.len();
                }
                if let Some(node_values) = nodes.get("nodeValue").and_then(Value::as_array) {
                    for value in node_values {
                        if let Some(text) = decode_string(&strings, value) {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() && summary.text_samples.len() < 12 {
                                summary.text_samples.insert(trimmed.to_string());
                            }
                        }
                    }
                }
                if let Some(attributes) = nodes.get("attributes").and_then(Value::as_array) {
                    for entry in attributes {
                        if let Some(attr_list) = entry.as_array() {
                            let mut iter = attr_list.iter();
                            while let Some(name_idx) = iter.next() {
                                let value_idx = iter.next();
                                if let Some(name) = decode_string(&strings, name_idx) {
                                    summary.attribute_keys.insert(name.to_lowercase());
                                }
                                if let Some(value_idx) = value_idx {
                                    if let Some(value) = decode_string(&strings, value_idx) {
                                        if !value.trim().is_empty()
                                            && summary.text_samples.len() < 12
                                        {
                                            summary.text_samples.insert(value.trim().to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    summary
}

fn summarize_ax(snapshot: &Value) -> AxSummary {
    let mut summary = AxSummary::default();

    if let Some(nodes) = snapshot.get("nodes").and_then(Value::as_array) {
        summary.node_count = nodes.len();
        for node in nodes {
            if let Some(role) = node.get("role").and_then(extract_role) {
                summary.roles.insert(role);
            }
            if let Some(actions) = node.get("actions").and_then(Value::as_array) {
                for action in actions {
                    if let Some(name) = action.get("name").and_then(Value::as_str) {
                        summary.actions.insert(name.to_string());
                    }
                }
            }
        }
    }

    summary
}

fn collect_dom_changes(before: &DomSummary, after: &DomSummary, changes: &mut Vec<Value>) {
    if before.node_count != after.node_count {
        changes.push(json!({
            "kind": "dom-node-count",
            "before": before.node_count,
            "after": after.node_count,
            "delta": (after.node_count as i64) - (before.node_count as i64),
        }));
    }

    let removed_attrs: Vec<_> = before
        .attribute_keys
        .difference(&after.attribute_keys)
        .take(8)
        .cloned()
        .collect();
    let added_attrs: Vec<_> = after
        .attribute_keys
        .difference(&before.attribute_keys)
        .take(8)
        .cloned()
        .collect();
    if !removed_attrs.is_empty() || !added_attrs.is_empty() {
        changes.push(json!({
            "kind": "dom-attribute-keys",
            "removed": removed_attrs,
            "added": added_attrs,
        }));
    }

    let removed_text: Vec<_> = before
        .text_samples
        .difference(&after.text_samples)
        .take(6)
        .cloned()
        .collect();
    let added_text: Vec<_> = after
        .text_samples
        .difference(&before.text_samples)
        .take(6)
        .cloned()
        .collect();
    if !removed_text.is_empty() || !added_text.is_empty() {
        changes.push(json!({
            "kind": "dom-text",
            "removed": removed_text,
            "added": added_text,
        }));
    }
}

fn collect_ax_changes(before: &AxSummary, after: &AxSummary, changes: &mut Vec<Value>) {
    if before.node_count != after.node_count {
        changes.push(json!({
            "kind": "ax-node-count",
            "before": before.node_count,
            "after": after.node_count,
            "delta": (after.node_count as i64) - (before.node_count as i64),
        }));
    }

    let removed_roles: Vec<_> = before
        .roles
        .difference(&after.roles)
        .take(8)
        .cloned()
        .collect();
    let added_roles: Vec<_> = after
        .roles
        .difference(&before.roles)
        .take(8)
        .cloned()
        .collect();
    if !removed_roles.is_empty() || !added_roles.is_empty() {
        changes.push(json!({
            "kind": "ax-roles",
            "removed": removed_roles,
            "added": added_roles,
        }));
    }

    let removed_actions: Vec<_> = before
        .actions
        .difference(&after.actions)
        .take(6)
        .cloned()
        .collect();
    let added_actions: Vec<_> = after
        .actions
        .difference(&before.actions)
        .take(6)
        .cloned()
        .collect();
    if !removed_actions.is_empty() || !added_actions.is_empty() {
        changes.push(json!({
            "kind": "ax-actions",
            "removed": removed_actions,
            "added": added_actions,
        }));
    }
}

fn decode_string(strings: &[Value], value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(num) => num.as_u64().and_then(|idx| {
            strings
                .get(idx as usize)
                .and_then(|entry| entry.as_str())
                .map(|s| s.to_string())
        }),
        _ => None,
    }
}

fn extract_role(value: &Value) -> Option<String> {
    match value {
        Value::String(role) => Some(role.to_lowercase()),
        Value::Object(obj) => obj
            .get("value")
            .and_then(Value::as_str)
            .map(|s| s.to_lowercase()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SnapLevel;
    use soulbrowser_core_types::{FrameId, PageId, SessionId};

    #[test]
    fn diff_reports_dom_and_ax_changes() {
        let page = PageId::new();
        let frame = FrameId::new();
        let session = SessionId::new();

        let base = DomAxSnapshot::new(
            page.clone(),
            frame.clone(),
            Some(session.clone()),
            SnapLevel::Full,
            json!({
                "strings": ["DIV", "class", "btn", "Submit"],
                "documents": [
                    {
                        "nodes": {
                            "nodeName": ["DIV"],
                            "nodeValue": ["Submit"],
                            "attributes": [["class", "btn"]],
                        }
                    }
                ]
            }),
            json!({
                "nodes": [
                    {
                        "role": { "value": "button" },
                        "actions": [{ "name": "focus" }]
                    }
                ]
            }),
        );

        let current = DomAxSnapshot::new(
            page,
            frame,
            Some(session),
            SnapLevel::Full,
            json!({
                "strings": ["DIV", "class", "btn", "Submit", "data-test"],
                "documents": [
                    {
                        "nodes": {
                            "nodeName": ["DIV", "SPAN"],
                            "nodeValue": ["Submit", ""],
                            "attributes": [
                                ["class", "btn"],
                            ["data-test", "example"],
                            ],
                        }
                    }
                ]
            }),
            json!({
                "nodes": [
                    {
                        "role": { "value": "link" },
                        "actions": [{ "name": "focus" }, { "name": "press" }]
                    }
                ]
            }),
        );

        let diff = diff(&base, &current);
        assert!(!diff.changes.is_empty());
        assert!(diff
            .changes
            .iter()
            .any(|change| change["kind"] == "dom-node-count"));
        assert!(diff
            .changes
            .iter()
            .any(|change| change["kind"] == "ax-roles"));
    }
}
