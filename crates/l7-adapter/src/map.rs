use std::time::Duration;

use serde_json::Value;
use soulbrowser_core_types::{RoutingHint, ToolCall as L1ToolCall};
use soulbrowser_scheduler::model::{CallOptions, DispatchRequest, Priority};

use crate::errors::{AdapterError, AdapterResult};
use crate::ports::ToolCall;

pub fn to_dispatch_request(call: &ToolCall) -> AdapterResult<DispatchRequest> {
    let payload = call.params.clone();
    let tool_call = L1ToolCall {
        call_id: call.trace_id.clone(),
        task_id: None,
        tool: call.tool.clone(),
        payload,
    };

    let timeout = Duration::from_millis(call.timeout_ms.max(1));
    let options = CallOptions {
        timeout,
        priority: priority_from_options(&call.options),
        interruptible: true,
        retry: Default::default(),
    };

    Ok(DispatchRequest {
        tool_call,
        options,
        routing_hint: routing_from_value(&call.routing)?,
    })
}

fn priority_from_options(options: &Value) -> Priority {
    if let Some(priority) = options.get("priority").and_then(Value::as_str) {
        match priority {
            "lightning" => Priority::Lightning,
            "quick" => Priority::Quick,
            "deep" => Priority::Deep,
            _ => Priority::Standard,
        }
    } else {
        Priority::Standard
    }
}

fn routing_from_value(value: &Value) -> AdapterResult<Option<RoutingHint>> {
    let json = match value {
        Value::Null => return Ok(None),
        Value::Object(map) if map.is_empty() => return Ok(None),
        other => other,
    };

    let obj = json.as_object().ok_or(AdapterError::InvalidArgument)?;
    let mut hint = RoutingHint::default();

    if let Some(session) = obj.get("session").and_then(Value::as_str) {
        hint.session = Some(soulbrowser_core_types::SessionId(session.to_string()));
    }
    if let Some(page) = obj.get("page").and_then(Value::as_str) {
        hint.page = Some(soulbrowser_core_types::PageId(page.to_string()));
    }
    if let Some(frame) = obj.get("frame").and_then(Value::as_str) {
        hint.frame = Some(soulbrowser_core_types::FrameId(frame.to_string()));
    }

    if let Some(prefer) = obj.get("prefer").and_then(Value::as_str) {
        hint.prefer = match prefer {
            "focused" => Some(soulbrowser_core_types::RoutePrefer::Focused),
            "recent_nav" => Some(soulbrowser_core_types::RoutePrefer::RecentNav),
            "main_frame" => Some(soulbrowser_core_types::RoutePrefer::MainFrame),
            _ => None,
        };
    }

    Ok(Some(hint))
}
