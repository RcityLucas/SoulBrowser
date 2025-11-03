use l6_privacy::{apply_obs, context::RedactCtx, RedactScope};
use tracing::debug;

use crate::ports::{ToolCall, ToolOutcome};

pub fn sanitize_tool_call(call: &mut ToolCall) {
    let mut ctx = RedactCtx {
        scope: RedactScope::Observation,
        trace_id: call.trace_id.clone(),
        ..Default::default()
    };
    ctx = ctx.with_tag(format!("tool={}", call.tool));

    let mut payload = call.params.clone();
    if let Err(error) = apply_obs(&mut payload, &ctx) {
        debug!(%error, "privacy sanitize request skipped");
    } else {
        call.params = payload;
    }
}

pub fn sanitize_tool_outcome(call: &ToolCall, outcome: &mut ToolOutcome) {
    if let Some(data) = outcome.data.as_mut() {
        let mut ctx = RedactCtx {
            scope: RedactScope::Observation,
            trace_id: outcome.trace_id.clone().or_else(|| call.trace_id.clone()),
            ..Default::default()
        };
        ctx = ctx.with_tag(format!("tool={}", call.tool));
        if let Err(error) = apply_obs(data, &ctx) {
            debug!(%error, "privacy sanitize response skipped");
        }
    }
}
