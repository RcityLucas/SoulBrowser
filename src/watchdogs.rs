use serde_json::Value;
use uuid::Uuid;

#[cfg(test)]
use chrono::Utc;

const PERMISSION_HINTS: &[&str] = &[
    "need your permission",
    "permission denied",
    "allow notifications",
    "需要授权",
    "权限请求",
];
const DOWNLOAD_HINTS: &[&str] = &[
    "downloading",
    "download complete",
    "save file",
    "正在下载",
    "已下载",
];

use crate::metrics::{record_download_prompt, record_permission_prompt};
use crate::observation::obstruction_from_entry;
use crate::task_status::{ObservationPayload, TaskAnnotation};

#[derive(Clone, Debug, serde::Serialize)]
pub struct WatchdogEvent {
    pub id: String,
    pub kind: String,
    pub severity: String,
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch_label: Option<String>,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug)]
pub struct WatchdogFinding {
    pub annotation: Option<TaskAnnotation>,
    pub event: WatchdogEvent,
}

/// Generate structured findings derived from passive watchdog checks.
pub fn analyze_observation(payload: &ObservationPayload) -> Vec<WatchdogFinding> {
    let mut findings = Vec::new();

    if let Some(kind) = obstruction_from_entry(&payload.artifact) {
        if let Some(annotation) = obstruction_annotation(payload, &kind) {
            findings.push(finding_from_annotation(annotation, payload));
        }
    }

    if let Some(annotation) = crash_annotation(payload) {
        findings.push(finding_from_annotation(annotation, payload));
    }
    if let Some(annotation) = permission_annotation(payload) {
        findings.push(finding_from_annotation(annotation, payload));
    }
    if let Some(annotation) = download_annotation(payload) {
        findings.push(finding_from_annotation(annotation, payload));
    }

    findings
}

fn finding_from_annotation(
    annotation: TaskAnnotation,
    payload: &ObservationPayload,
) -> WatchdogFinding {
    let event = WatchdogEvent {
        id: annotation.id.clone(),
        kind: annotation
            .kind
            .clone()
            .unwrap_or_else(|| "watchdog".to_string()),
        severity: annotation
            .severity
            .clone()
            .unwrap_or_else(|| "info".to_string()),
        note: annotation.note.clone(),
        step_id: annotation.step_id.clone(),
        dispatch_label: annotation.dispatch_label.clone(),
        recorded_at: payload.recorded_at,
    };

    WatchdogFinding {
        annotation: Some(annotation),
        event,
    }
}

fn obstruction_annotation(payload: &ObservationPayload, kind: &str) -> Option<TaskAnnotation> {
    let (severity, note) = match kind {
        "consent_gate" => (
            "info",
            "检测到站点弹出隐私/同意提示，可能阻塞后续操作".to_string(),
        ),
        "captcha" => (
            "critical",
            "检测到 CAPTCHA/验证码，需人工介入或切换策略".to_string(),
        ),
        "unusual_traffic" => (
            "warn",
            "站点返回异常流量/自动化拦截提示，建议切换备选站点".to_string(),
        ),
        "login_wall" => (
            "warn",
            "页面被登录要求阻塞，需提供凭据或跳过该站点".to_string(),
        ),
        "blank_page" => (
            "warn",
            "页面出现 about:blank 或空白响应，考虑刷新或回退".to_string(),
        ),
        _ => return None,
    };

    Some(TaskAnnotation {
        id: format!("watchdog-{}-{}", kind, Uuid::new_v4()),
        step_id: payload.step_id.clone(),
        dispatch_label: payload.dispatch_label.clone(),
        note,
        bbox: payload
            .bbox
            .clone()
            .or_else(|| extract_bbox(&payload.artifact)),
        author: Some("watchdog".to_string()),
        severity: Some(severity.to_string()),
        kind: Some(kind.to_string()),
        created_at: payload.recorded_at,
    })
}

fn aggregate_text(value: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(data) = value.get("data") {
        if let Some(sample) = data.get("text_sample").and_then(Value::as_str) {
            parts.push(sample.to_ascii_lowercase());
        }
        if let Some(identity) = data.get("identity").and_then(Value::as_str) {
            parts.push(identity.to_ascii_lowercase());
        }
        if let Some(note) = data.get("note").and_then(Value::as_str) {
            parts.push(note.to_ascii_lowercase());
        }
    }
    parts.join(" ")
}

fn extract_bbox(artifact: &Value) -> Option<Value> {
    artifact.get("bbox").cloned()
}

fn permission_annotation(payload: &ObservationPayload) -> Option<TaskAnnotation> {
    let blob = aggregate_text(&payload.artifact);
    if blob.is_empty() {
        return None;
    }
    if PERMISSION_HINTS.iter().any(|hint| blob.contains(hint)) {
        record_permission_prompt();
        return Some(TaskAnnotation {
            id: format!("watchdog-permission-{}", Uuid::new_v4()),
            step_id: payload.step_id.clone(),
            dispatch_label: payload.dispatch_label.clone(),
            note: "检测到浏览器权限弹窗/请求，请授权或切换方案".to_string(),
            bbox: payload.bbox.clone(),
            author: Some("watchdog".to_string()),
            severity: Some("warn".to_string()),
            kind: Some("permission_request".to_string()),
            created_at: payload.recorded_at,
        });
    }
    None
}

fn download_annotation(payload: &ObservationPayload) -> Option<TaskAnnotation> {
    let blob = aggregate_text(&payload.artifact);
    if blob.is_empty() {
        return None;
    }
    if DOWNLOAD_HINTS.iter().any(|hint| blob.contains(hint)) {
        record_download_prompt();
        return Some(TaskAnnotation {
            id: format!("watchdog-download-{}", Uuid::new_v4()),
            step_id: payload.step_id.clone(),
            dispatch_label: payload.dispatch_label.clone(),
            note: "检测到下载提示，自动化可能被阻塞".to_string(),
            bbox: payload.bbox.clone(),
            author: Some("watchdog".to_string()),
            severity: Some("info".to_string()),
            kind: Some("download_prompt".to_string()),
            created_at: payload.recorded_at,
        });
    }
    None
}

fn crash_annotation(payload: &ObservationPayload) -> Option<TaskAnnotation> {
    let err_text = payload
        .artifact
        .get("error")
        .and_then(Value::as_str)
        .map(|s| s.to_ascii_lowercase());
    let msg = err_text.as_deref()?;
    let crash_hints = ["page crashed", "target closed", "crash", "renderer"];
    if !crash_hints.iter().any(|hint| msg.contains(hint)) {
        return None;
    }
    Some(TaskAnnotation {
        id: format!("watchdog-crash-{}", Uuid::new_v4()),
        step_id: payload.step_id.clone(),
        dispatch_label: payload.dispatch_label.clone(),
        note: "浏览器会话异常终止，检测到页面崩溃/Target closed".to_string(),
        bbox: None,
        author: Some("watchdog".to_string()),
        severity: Some("critical".to_string()),
        kind: Some("page_crash".to_string()),
        created_at: payload.recorded_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_status::ObservationPayload;
    use serde_json::json;

    #[test]
    fn obstruction_creates_annotation() {
        let payload = ObservationPayload {
            observation_type: "artifact".to_string(),
            task_id: "task".to_string(),
            step_id: Some("step-1".to_string()),
            dispatch_label: Some("observe".to_string()),
            dispatch_index: Some(0),
            screenshot_path: None,
            bbox: None,
            content_type: None,
            recorded_at: Utc::now(),
            artifact: json!({
                "data": {
                    "identity": "Example",
                    "text_sample": "Please complete the CAPTCHA"
                }
            }),
        };

        let annotations = analyze_observation(&payload);
        assert_eq!(annotations.len(), 1);
        let ann = annotations[0].annotation.as_ref().unwrap();
        assert_eq!(ann.step_id.as_deref(), Some("step-1"));
        assert_eq!(ann.severity.as_deref(), Some("critical"));
        assert_eq!(ann.kind.as_deref(), Some("captcha"));
        assert!(ann.note.contains("CAPTCHA"));
        assert_eq!(annotations[0].event.kind, "captcha");
    }

    #[test]
    fn crash_error_creates_annotation() {
        let payload = ObservationPayload {
            observation_type: "artifact".to_string(),
            task_id: "task".to_string(),
            step_id: None,
            dispatch_label: None,
            dispatch_index: None,
            screenshot_path: None,
            bbox: None,
            content_type: None,
            recorded_at: Utc::now(),
            artifact: json!({
                "error": "Target closed: page crashed"
            }),
        };
        let annotations = analyze_observation(&payload);
        assert!(annotations.iter().any(|finding| {
            finding.annotation.as_ref().unwrap().kind.as_deref() == Some("page_crash")
        }));
    }

    #[test]
    fn detects_permission_annotation() {
        let payload = ObservationPayload {
            observation_type: "artifact".to_string(),
            task_id: "task".to_string(),
            step_id: Some("step".to_string()),
            dispatch_label: None,
            dispatch_index: None,
            screenshot_path: None,
            bbox: None,
            content_type: None,
            recorded_at: Utc::now(),
            artifact: json!({
                "data": {
                    "text_sample": "Please allow notifications"
                }
            }),
        };
        let annotations = analyze_observation(&payload);
        assert!(annotations.iter().any(|finding| {
            finding.annotation.as_ref().unwrap().kind.as_deref() == Some("permission_request")
        }));
    }

    #[test]
    fn detects_download_annotation() {
        let payload = ObservationPayload {
            observation_type: "artifact".to_string(),
            task_id: "task".to_string(),
            step_id: Some("step".to_string()),
            dispatch_label: None,
            dispatch_index: None,
            screenshot_path: None,
            bbox: None,
            content_type: None,
            recorded_at: Utc::now(),
            artifact: json!({
                "data": {
                    "text_sample": "Downloading file"
                }
            }),
        };
        let annotations = analyze_observation(&payload);
        assert!(annotations.iter().any(|finding| {
            finding.annotation.as_ref().unwrap().kind.as_deref() == Some("download_prompt")
        }));
    }
}
