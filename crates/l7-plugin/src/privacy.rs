use l6_privacy::{apply_obs, context::RedactCtx, RedactScope};
use serde_json::Value;
use tracing::debug;

pub fn redact_payload(payload: Value, trace_id: Option<String>) -> Value {
    let ctx = RedactCtx {
        scope: RedactScope::Observation,
        trace_id,
        ..Default::default()
    };
    let mut value = payload;
    if let Err(error) = apply_obs(&mut value, &ctx) {
        debug!(%error, "plugin payload redaction skipped");
    }
    value
}
