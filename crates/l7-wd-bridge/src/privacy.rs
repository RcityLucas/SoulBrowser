use l6_privacy::{apply_obs, context::RedactCtx, RedactScope};
use serde_json::Value;
use tracing::debug;

pub fn sanitize_response(mut value: Value, trace_id: Option<String>) -> Value {
    let ctx = RedactCtx {
        scope: RedactScope::Observation,
        trace_id,
        ..Default::default()
    };
    if let Err(error) = apply_obs(&mut value, &ctx) {
        debug!(%error, "wd bridge response privacy skipped");
        value
    } else {
        value
    }
}
