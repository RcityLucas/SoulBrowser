use std::collections::BTreeSet;

use serde_json::{json, Map as JsonMap, Value};

use crate::model::{AnchorDescriptor, JudgeReport};
use crate::policy::JudgePolicy;

pub fn visible(anchor: &AnchorDescriptor, policy: &JudgePolicy) -> JudgeReport {
    let mut facts = JsonMap::new();
    let mut issues = Vec::new();

    match &anchor.geometry {
        Some(geom) => {
            let area = (geom.width.max(0.0) * geom.height.max(0.0)).round();
            facts.insert(
                "geometry".into(),
                json!({
                    "x": geom.x,
                    "y": geom.y,
                    "width": geom.width,
                    "height": geom.height,
                    "area": area,
                }),
            );
            if area <= 1.0 {
                issues.push("zero_area".into());
            }
            if let Some(min_area) = policy.minimum_visible_area {
                if area < min_area {
                    issues.push(format!("area<{:.0}", min_area));
                }
            }
        }
        None => {
            issues.push("missing_geometry".into());
        }
    }

    let mut style_flags = Vec::new();
    let mut style_hints = StyleHints::default();
    if let Some(attrs) = attributes(anchor) {
        if attr_flag_true(attrs, "hidden") {
            issues.push("hidden_attribute".into());
        }
        if attr_flag_true(attrs, "aria-hidden") {
            issues.push("aria_hidden".into());
        }
        if let Some(style) = attr_string(attrs, "style") {
            style_hints.merge(inspect_inline_style(&style));
        }
        if let Some(class_attr) = attr_string(attrs, "class") {
            facts.insert("class".into(), json!(class_attr));
        }
    }
    if let Some(style_map) = computed_style(anchor) {
        style_hints.merge(inspect_style_map(style_map));
    }
    if style_hints.hides {
        issues.push("style_hidden".into());
    }
    if style_hints.zero_opacity {
        issues.push("opacity_zero".into());
    }
    if let (Some(min_opacity), Some(current_opacity)) =
        (policy.minimum_opacity, style_hints.opacity)
    {
        if current_opacity < min_opacity {
            issues.push(format!("opacity<{:.2}", min_opacity));
        }
        facts.insert("opacity".into(), json!(current_opacity));
    } else if let Some(opacity) = style_hints.opacity {
        facts.insert("opacity".into(), json!(opacity));
    }
    style_flags.extend(style_hints.flags);
    if !style_flags.is_empty() {
        style_flags.sort();
        style_flags.dedup();
        facts.insert("style_flags".into(), json!(style_flags));
    }

    if let Some(role) = ax_role(anchor) {
        facts.insert("ax_role".into(), json!(role));
    }
    if let Some(name) = ax_name(anchor) {
        facts.insert("ax_name".into(), json!(name));
    }
    let ax_states = ax_states(anchor);
    if !ax_states.is_empty() {
        if ax_states
            .iter()
            .any(|state| matches!(state.as_str(), "invisible" | "hidden" | "offscreen"))
        {
            issues.push("ax_hidden".into());
        }
        facts.insert("ax_states".into(), json!(ax_states));
    }

    let ok = issues.is_empty();
    if !issues.is_empty() {
        facts.insert("issues".into(), json!(issues.clone()));
    }

    let reason = format_reason(if ok { "visible" } else { "not_visible" }, &issues);

    JudgeReport {
        ok,
        reason,
        facts: Value::Object(facts),
    }
}

pub fn clickable(anchor: &AnchorDescriptor, policy: &JudgePolicy) -> JudgeReport {
    let visibility = visible(anchor, policy);

    let mut facts = JsonMap::new();
    facts.insert("visibility".into(), visibility.facts.clone());

    let mut issues = Vec::new();
    if !visibility.ok {
        issues.push("not_visible".into());
    }

    let attrs = attributes(anchor);
    let node_name = node_name(anchor);
    let roles = roles(anchor, attrs);
    let ax_role = ax_role(anchor);
    let ax_states = ax_states(anchor);

    let actionable_node = node_name
        .as_deref()
        .map(|name| {
            matches!(
                name,
                "BUTTON" | "A" | "AREA" | "INPUT" | "SUMMARY" | "SELECT" | "TEXTAREA"
            )
        })
        .unwrap_or(false);
    let has_href = attrs
        .as_ref()
        .map_or(false, |map| attr_present(map, "href"));
    let has_onclick = attrs.as_ref().map_or(false, |map| {
        attr_present(map, "onclick") || attr_present(map, "onClick")
    });
    let actionable_role = roles.iter().any(|role| {
        matches!(
            role.as_str(),
            "button" | "link" | "menuitem" | "tab" | "checkbox" | "radio"
        )
    });
    let actionable_ax_role = ax_role
        .as_deref()
        .map(|role| {
            matches!(
                role,
                "button" | "link" | "menuitem" | "menuItem" | "tab" | "checkbox" | "radio"
            )
        })
        .unwrap_or(false);
    let actionable_input = attrs
        .as_ref()
        .map_or(false, |map| match attr_string(map, "type") {
            Some(ref ty) => matches!(
                ty.as_str(),
                "submit" | "button" | "reset" | "checkbox" | "radio"
            ),
            None => false,
        });

    let mut style_hints = StyleHints::default();
    if let Some(map) = attrs.as_ref() {
        if let Some(style) = attr_string(map, "style") {
            style_hints.merge(inspect_inline_style(&style));
        }
    }
    if let Some(style_map) = computed_style(anchor) {
        style_hints.merge(inspect_style_map(style_map));
    }
    let pointer_blocked = style_hints.pointer_blocked;
    if pointer_blocked && policy.pointer_events_block {
        issues.push("pointer_events_none".into());
    }

    let disabled = attrs.as_ref().map_or(false, |map| is_disabled(map));
    let ax_disabled = ax_states.iter().any(|state| state == "disabled");
    if disabled {
        issues.push("disabled".into());
    }
    if ax_disabled {
        issues.push("ax_disabled".into());
    }

    let actionable = actionable_node
        || actionable_role
        || actionable_ax_role
        || has_href
        || has_onclick
        || actionable_input;
    if !actionable {
        issues.push("no_click_signal".into());
    }

    let ok = visibility.ok && actionable && !disabled && !ax_disabled && !pointer_blocked;

    facts.insert(
        "node".into(),
        json!({
            "name": node_name,
            "roles": roles,
            "signals": {
                "actionable_node": actionable_node,
                "actionable_role": actionable_role,
                "actionable_ax_role": actionable_ax_role,
                "has_href": has_href,
                "has_onclick": has_onclick,
                "pointer_blocked": pointer_blocked,
            }
        }),
    );
    facts.insert(
        "ax".into(),
        json!({
            "role": ax_role,
            "states": ax_states,
        }),
    );
    if let Some(attrs) = attrs {
        facts.insert(
            "attributes".into(),
            json!({
                "disabled": attrs.get("disabled"),
                "aria-disabled": attrs.get("aria-disabled"),
                "tabindex": attrs.get("tabindex"),
            }),
        );
    }

    if !issues.is_empty() {
        facts.insert("issues".into(), json!(issues.clone()));
    }

    let reason = format_reason(if ok { "clickable" } else { "not_clickable" }, &issues);

    JudgeReport {
        ok,
        reason,
        facts: Value::Object(facts),
    }
}

pub fn enabled(anchor: &AnchorDescriptor, _policy: &JudgePolicy) -> JudgeReport {
    let mut facts = JsonMap::new();
    let mut issues = Vec::new();

    let attrs = attributes(anchor);
    let disabled = attrs.as_ref().map_or(false, |map| is_disabled(map));
    let ax_states = ax_states(anchor);
    let ax_disabled = ax_states.iter().any(|state| state == "disabled");
    let ax_readonly = ax_states.iter().any(|state| state == "readonly");
    if disabled {
        issues.push("disabled".into());
    }
    if ax_disabled {
        issues.push("ax_disabled".into());
    }

    let readonly = attrs
        .as_ref()
        .map_or(false, |map| attr_flag_true(map, "readonly"));
    if readonly {
        issues.push("readonly".into());
    }
    if ax_readonly {
        issues.push("ax_readonly".into());
    }

    if let Some(attrs) = attrs {
        facts.insert(
            "attributes".into(),
            json!({
                "disabled": attrs.get("disabled"),
                "aria-disabled": attrs.get("aria-disabled"),
                "readonly": attrs.get("readonly"),
            }),
        );
    }
    if !ax_states.is_empty() {
        facts.insert("ax_states".into(), json!(ax_states));
    }
    if let Some(name) = node_name(anchor) {
        facts.insert("node_name".into(), json!(name));
    }

    if !issues.is_empty() {
        facts.insert("issues".into(), json!(issues.clone()));
    }

    let ok = !disabled && !ax_disabled;
    let reason = format_reason(if ok { "enabled" } else { "disabled" }, &issues);

    JudgeReport {
        ok,
        reason,
        facts: Value::Object(facts),
    }
}

fn roles(anchor: &AnchorDescriptor, attrs: Option<&JsonMap<String, Value>>) -> Vec<String> {
    let mut accumulator = BTreeSet::new();
    if let Some(object) = anchor.value.as_object() {
        if let Some(Value::String(role)) = object.get("role") {
            accumulator.insert(role.to_lowercase());
        }
    }
    if let Some(attrs) = attrs {
        if let Some(role_value) = attr_string(attrs, "role") {
            for role in role_value.split_whitespace() {
                if !role.is_empty() {
                    accumulator.insert(role.to_lowercase());
                }
            }
        }
    }
    accumulator.into_iter().collect()
}

fn node_name(anchor: &AnchorDescriptor) -> Option<String> {
    anchor
        .value
        .as_object()
        .and_then(|obj| obj.get("nodeName"))
        .and_then(value_to_string)
        .map(|name| name.to_ascii_uppercase())
}

fn attributes(anchor: &AnchorDescriptor) -> Option<&JsonMap<String, Value>> {
    anchor
        .value
        .as_object()
        .and_then(|obj| obj.get("attributes"))
        .and_then(Value::as_object)
}

fn computed_style(anchor: &AnchorDescriptor) -> Option<&JsonMap<String, Value>> {
    anchor
        .value
        .as_object()
        .and_then(|obj| obj.get("computedStyle"))
        .and_then(Value::as_object)
}

fn attr_string(attrs: &JsonMap<String, Value>, key: &str) -> Option<String> {
    attrs.get(key).and_then(value_to_string)
}

fn attr_present(attrs: &JsonMap<String, Value>, key: &str) -> bool {
    attrs.contains_key(key)
}

fn attr_flag_true(attrs: &JsonMap<String, Value>, key: &str) -> bool {
    attrs.get(key).map_or(false, |value| match value {
        Value::Bool(flag) => *flag,
        Value::Number(num) => num.as_i64().map(|v| v != 0).unwrap_or(false),
        Value::String(s) => {
            let normalized = s.trim().to_ascii_lowercase();
            normalized.is_empty() || matches!(normalized.as_str(), "true" | "1" | "yes" | "on")
        }
        Value::Null => true,
        _ => false,
    })
}

fn ax_role(anchor: &AnchorDescriptor) -> Option<String> {
    anchor
        .value
        .as_object()
        .and_then(|obj| obj.get("axRole"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}

fn ax_name(anchor: &AnchorDescriptor) -> Option<String> {
    anchor
        .value
        .as_object()
        .and_then(|obj| obj.get("axName"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}

fn ax_states(anchor: &AnchorDescriptor) -> Vec<String> {
    anchor
        .value
        .as_object()
        .and_then(|obj| obj.get("axStates"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|value| value.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(num) => num.as_i64().map(|n| n.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn is_disabled(attrs: &JsonMap<String, Value>) -> bool {
    attr_flag_true(attrs, "disabled") || attr_flag_true(attrs, "aria-disabled")
}

fn format_reason(base: &str, issues: &[String]) -> String {
    if issues.is_empty() {
        base.to_string()
    } else {
        format!("{}({})", base, issues.join(","))
    }
}

#[derive(Default)]
struct StyleHints {
    flags: Vec<String>,
    hides: bool,
    zero_opacity: bool,
    opacity: Option<f32>,
    pointer_blocked: bool,
}

impl StyleHints {
    fn merge(&mut self, other: StyleHints) {
        self.flags.extend(other.flags);
        self.hides |= other.hides;
        self.zero_opacity |= other.zero_opacity;
        if self.opacity.is_none() {
            self.opacity = other.opacity;
        }
        self.pointer_blocked |= other.pointer_blocked;
    }
}

fn inspect_inline_style(style: &str) -> StyleHints {
    let lower = style.to_ascii_lowercase();
    let mut flags = Vec::new();
    let mut hides = false;
    let mut zero_opacity = false;
    let mut opacity: Option<f32> = None;
    let mut pointer_blocked = false;

    for chunk in lower.split(';') {
        let entry = chunk.trim();
        if entry.is_empty() {
            continue;
        }
        if entry.contains("display:none") {
            hides = true;
            flags.push("display:none".into());
        }
        if entry.contains("visibility:hidden") {
            hides = true;
            flags.push("visibility:hidden".into());
        }
        if entry.contains("pointer-events:none") {
            pointer_blocked = true;
            flags.push("pointer-events:none".into());
        }
        if let Some(rest) = entry.strip_prefix("opacity:") {
            if let Ok(value) = rest.trim().parse::<f32>() {
                opacity = Some(value);
                if value <= 0.0 {
                    zero_opacity = true;
                }
            }
        }
    }

    if zero_opacity && !flags.iter().any(|flag| flag == "opacity:0") {
        flags.push("opacity:0".into());
    }
    if lower.contains("clip-path") || lower.contains("clip:") {
        flags.push("clip".into());
    }

    StyleHints {
        flags,
        hides,
        zero_opacity,
        opacity,
        pointer_blocked,
    }
}

fn inspect_style_map(map: &JsonMap<String, Value>) -> StyleHints {
    let mut hints = StyleHints::default();

    if let Some(display) = map.get("display").and_then(value_to_string) {
        if display.trim().eq_ignore_ascii_case("none") {
            hints.hides = true;
            hints.flags.push("display:none".into());
        }
    }
    if let Some(visibility) = map.get("visibility").and_then(value_to_string) {
        if visibility.trim().eq_ignore_ascii_case("hidden") {
            hints.hides = true;
            hints.flags.push("visibility:hidden".into());
        }
    }
    if let Some(opacity_value) = map.get("opacity").and_then(value_to_string) {
        if let Ok(value) = opacity_value.trim().parse::<f32>() {
            hints.opacity = Some(value);
            if value <= 0.0 {
                hints.zero_opacity = true;
                hints.flags.push("opacity:0".into());
            }
        }
    }
    if let Some(pointer) = map.get("pointer-events").and_then(value_to_string) {
        if pointer.trim().eq_ignore_ascii_case("none") {
            hints.pointer_blocked = true;
            hints.flags.push("pointer-events:none".into());
        }
    }
    if map.contains_key("clip-path") || map.contains_key("clip") {
        hints.flags.push("clip".into());
    }

    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AnchorGeometry;
    use serde_json::{json, Map as JsonMap, Value};
    use soulbrowser_core_types::FrameId;

    fn anchor_with(
        node_name: &str,
        attributes: JsonMap<String, Value>,
        geometry: Option<AnchorGeometry>,
    ) -> AnchorDescriptor {
        AnchorDescriptor {
            strategy: "test".into(),
            value: json!({
                "nodeName": node_name,
                "attributes": attributes,
            }),
            frame_id: FrameId::new(),
            confidence: 0.5,
            backend_node_id: None,
            geometry,
        }
    }

    fn anchor_with_ax(
        node_name: &str,
        attributes: JsonMap<String, Value>,
        ax_role: Option<&str>,
        ax_states: &[&str],
    ) -> AnchorDescriptor {
        AnchorDescriptor {
            strategy: "test".into(),
            value: json!({
                "nodeName": node_name,
                "attributes": attributes,
                "axRole": ax_role,
                "axStates": ax_states,
            }),
            frame_id: FrameId::new(),
            confidence: 0.5,
            backend_node_id: None,
            geometry: Some(geometry()),
        }
    }

    fn anchor_with_computed(
        node_name: &str,
        attributes: JsonMap<String, Value>,
        computed_style: JsonMap<String, Value>,
        geometry: Option<AnchorGeometry>,
    ) -> AnchorDescriptor {
        AnchorDescriptor {
            strategy: "test".into(),
            value: json!({
                "nodeName": node_name,
                "attributes": attributes,
                "computedStyle": computed_style,
            }),
            frame_id: FrameId::new(),
            confidence: 0.5,
            backend_node_id: None,
            geometry,
        }
    }

    fn geometry() -> AnchorGeometry {
        AnchorGeometry {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 20.0,
        }
    }

    #[test]
    fn visible_detects_hidden_attribute() {
        let mut attrs = JsonMap::new();
        attrs.insert("hidden".into(), Value::String(String::new()));
        let anchor = anchor_with("DIV", attrs, Some(geometry()));
        let report = visible(&anchor, &JudgePolicy::default());
        assert!(!report.ok);
        assert!(report.reason.contains("not_visible"));
    }

    #[test]
    fn clickable_requires_actionable_signal() {
        let anchor = anchor_with("DIV", JsonMap::new(), Some(geometry()));
        let report = clickable(&anchor, &JudgePolicy::default());
        assert!(!report.ok);

        let mut attrs = JsonMap::new();
        attrs.insert("href".into(), Value::String("/next".into()));
        let anchor_link = anchor_with("A", attrs, Some(geometry()));
        let report_link = clickable(&anchor_link, &JudgePolicy::default());
        assert!(report_link.ok);
    }

    #[test]
    fn clickable_uses_ax_role() {
        let anchor = anchor_with_ax("DIV", JsonMap::new(), Some("button"), &[]);
        let report = clickable(&anchor, &JudgePolicy::default());
        assert!(report.ok);
    }

    #[test]
    fn enabled_checks_disabled_state() {
        let mut attrs = JsonMap::new();
        attrs.insert("disabled".into(), Value::String(String::new()));
        let anchor = anchor_with("BUTTON", attrs, Some(geometry()));
        let report = enabled(&anchor, &JudgePolicy::default());
        assert!(!report.ok);
        assert!(report.reason.contains("disabled"));
    }

    #[test]
    fn enabled_checks_ax_state() {
        let anchor = anchor_with_ax("BUTTON", JsonMap::new(), None, &["disabled"]);
        let report = enabled(&anchor, &JudgePolicy::default());
        assert!(!report.ok);
        assert!(report.reason.contains("disabled"));
    }

    #[test]
    fn visible_uses_computed_style_hidden() {
        let attrs = JsonMap::new();
        let mut computed = JsonMap::new();
        computed.insert("display".into(), Value::String("none".into()));
        let anchor = anchor_with_computed("DIV", attrs, computed, Some(geometry()));
        let report = visible(&anchor, &JudgePolicy::default());
        assert!(!report.ok);
        assert!(report.reason.contains("not_visible"));
    }

    #[test]
    fn clickable_blocks_pointer_events_none() {
        let mut attrs = JsonMap::new();
        attrs.insert("style".into(), Value::String("pointer-events:none".into()));
        let anchor = anchor_with("BUTTON", attrs, Some(geometry()));
        let report = clickable(&anchor, &JudgePolicy::default());
        assert!(!report.ok);
        assert!(report.reason.contains("not_clickable"));
    }

    #[test]
    fn clickable_blocks_pointer_events_from_computed_style() {
        let attrs = JsonMap::new();
        let mut computed = JsonMap::new();
        computed.insert("pointer-events".into(), Value::String("none".into()));
        let anchor = anchor_with_computed("BUTTON", attrs, computed, Some(geometry()));
        let report = clickable(&anchor, &JudgePolicy::default());
        assert!(!report.ok);
        assert!(report.reason.contains("not_clickable"));
    }

    #[test]
    fn visible_flags_ax_hidden_state() {
        let anchor = anchor_with_ax("DIV", JsonMap::new(), Some("link"), &["hidden"]);
        let report = visible(&anchor, &JudgePolicy::default());
        assert!(!report.ok);
        assert!(report.reason.contains("not_visible"));
    }
}
