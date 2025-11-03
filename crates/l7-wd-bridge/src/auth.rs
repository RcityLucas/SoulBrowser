use axum::http::HeaderMap;

use crate::errors::{BridgeError, BridgeResult};
use crate::policy::WebDriverBridgePolicy;

pub fn authenticate(headers: &HeaderMap, policy: &WebDriverBridgePolicy) -> BridgeResult<String> {
    if !policy.enabled {
        return Err(BridgeError::Disabled);
    }

    let tenant = headers
        .get("x-tenant-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or(BridgeError::Unauthorized)?;

    let allowed = policy.tenants.iter().any(|t| t.id == tenant && t.enable);
    if !allowed {
        return Err(BridgeError::Forbidden);
    }

    Ok(tenant)
}
