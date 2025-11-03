use serde_json::Value;
use soulbrowser_core_types::{RoutingHint, ToolCall as L1ToolCall};

use crate::errors::{BridgeError, BridgeResult};

pub fn to_tool_call(method: &str, payload: Value) -> BridgeResult<L1ToolCall> {
    let tool = match method {
        "navigateTo" => "navigate-to-url",
        "clickElement" => "click",
        "getElementText" => "get-text",
        "getElementAttribute" => "get-attribute",
        "getTitle" => "get-title",
        _ => return Err(BridgeError::NotImplemented),
    };

    Ok(L1ToolCall {
        call_id: None,
        task_id: None,
        tool: tool.to_string(),
        payload,
    })
}

pub fn to_routing(_session_id: &str) -> Option<RoutingHint> {
    None
}
