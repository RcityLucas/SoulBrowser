use crate::context::RedactCtx;
use crate::errors::PrivacyResult;
use crate::policy::{PrivacyPolicyHandle, PrivacyPolicyView};
use crate::text::{digest, mask_pii};
use crate::url::redact_url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RedactReport {
    pub applied: bool,
    pub fields: Vec<String>,
    pub reason: Option<String>,
}

impl RedactReport {
    pub fn skipped(reason: &str) -> Self {
        Self {
            applied: false,
            fields: vec![],
            reason: Some(reason.to_string()),
        }
    }

    fn applied(fields: Vec<String>) -> Self {
        Self {
            applied: true,
            fields,
            reason: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShotMeta {
    pub regions: Vec<String>,
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ImageBuf {
    pub width: u32,
    pub height: u32,
}

pub fn apply_obs(obs: &mut JsonValue, ctx: &RedactCtx) -> PrivacyResult<RedactReport> {
    apply_json_value(obs, ctx, "observation")
}

pub fn apply_event(event: &mut JsonValue, ctx: &RedactCtx) -> PrivacyResult<RedactReport> {
    apply_json_value(event, ctx, "event")
}

pub fn apply_sc_light(event: &mut JsonValue, ctx: &RedactCtx) -> PrivacyResult<RedactReport> {
    apply_json_value(event, ctx, "state_center")
}

pub fn apply_export(line: &mut JsonValue, ctx: &RedactCtx) -> PrivacyResult<RedactReport> {
    apply_json_value(line, ctx, "export")
}

pub fn apply_screenshot(
    meta: &mut ShotMeta,
    bitmap: &mut ImageBuf,
    ctx: &RedactCtx,
) -> PrivacyResult<RedactReport> {
    let handle = PrivacyPolicyHandle::global();
    if !handle.enabled_for(ctx) {
        return Ok(RedactReport::skipped("policy disabled"));
    }

    let policy = handle.snapshot();
    if !policy.screenshot_enable {
        return Ok(RedactReport::skipped("screenshot policy disabled"));
    }

    if meta.regions.is_empty() {
        meta.regions
            .extend(policy.screenshot_rules.iter().map(|rule| rule.name.clone()));
    }
    if meta.regions.is_empty() {
        meta.regions.push("viewport".into());
    }
    meta.mode = Some(policy.screenshot_mode.as_str().to_string());

    if bitmap.width == 0 || bitmap.height == 0 {
        bitmap.width = 1;
        bitmap.height = 1;
    }

    Ok(RedactReport::applied(vec![format!(
        "screenshot.{}",
        policy.screenshot_mode.as_str()
    )]))
}

fn apply_json_value(
    value: &mut JsonValue,
    ctx: &RedactCtx,
    reason: &str,
) -> PrivacyResult<RedactReport> {
    let handle = PrivacyPolicyHandle::global();
    if !handle.enabled_for(ctx) {
        return Ok(RedactReport::skipped("policy disabled"));
    }

    let policy = handle.snapshot();
    let mut affected = Vec::new();
    redact_value("".to_string(), value, &policy, &mut affected)?;

    if affected.is_empty() {
        Ok(RedactReport::skipped(reason))
    } else {
        Ok(RedactReport::applied(affected))
    }
}

fn redact_value(
    path: String,
    value: &mut JsonValue,
    policy: &PrivacyPolicyView,
    affected: &mut Vec<String>,
) -> PrivacyResult<()> {
    match value {
        JsonValue::String(current) => {
            let masked = mask_pii(current, &policy.pii_patterns);
            let (hash, len) = digest(
                &masked,
                policy.text_hash_alg.clone(),
                policy.message_max_len,
            );
            *value = json!({ "hash": hash, "len": len });
            affected.push(path);
            Ok(())
        }
        JsonValue::Object(map) => {
            for (key, val) in map.iter_mut() {
                let nested_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", path, key)
                };
                if key.ends_with("_url") || key == "href" || key == "src" {
                    if let Some(raw) = val.as_str() {
                        let redacted = redact_url(raw, &policy.query_allow_keys);
                        *val = JsonValue::String(redacted);
                        affected.push(nested_path.clone());
                        continue;
                    }
                }
                if attr_whitelisted(policy, key) {
                    continue;
                }
                redact_value(nested_path, val, policy, affected)?;
            }
            Ok(())
        }
        JsonValue::Array(items) => {
            for (idx, item) in items.iter_mut().enumerate() {
                let nested_path = if path.is_empty() {
                    format!("[{}]", idx)
                } else {
                    format!("{}[{}]", path, idx)
                };
                redact_value(nested_path, item, policy, affected)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn attr_whitelisted(policy: &PrivacyPolicyView, key: &str) -> bool {
    policy
        .attrs_whitelist
        .iter()
        .any(|allowed| match allowed.strip_suffix('*') {
            Some(prefix) => key.starts_with(prefix),
            None => allowed == key,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::RedactScope;
    use crate::policy::{set_policy, PrivacyPolicyView, ShotMode, ShotRule};

    #[test]
    fn screenshot_rules_populate_regions() {
        let mut view = PrivacyPolicyView::default();
        view.enable = true;
        view.screenshot_enable = true;
        view.screenshot_mode = ShotMode::Blur;
        view.screenshot_rules = vec![ShotRule {
            name: "input.password".into(),
            selector: "input[type=password]".into(),
        }];
        set_policy(view);

        let mut meta = ShotMeta::default();
        let mut buf = ImageBuf::default();
        let ctx = RedactCtx {
            scope: RedactScope::Screenshot,
            export: true,
            ..Default::default()
        };

        let report = apply_screenshot(&mut meta, &mut buf, &ctx).expect("screenshot redaction");
        assert!(report.applied);
        assert_eq!(meta.mode.as_deref(), Some("blur"));
        assert_eq!(meta.regions, vec!["input.password".to_string()]);
        assert!(buf.width > 0 && buf.height > 0);
    }
}
