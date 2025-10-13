#![allow(dead_code)]

use soulbrowser_core_types::{ExecRoute, RoutingHint, SoulError};

pub fn resolve_route(_hint: Option<RoutingHint>) -> Result<ExecRoute, SoulError> {
    Err(SoulError::new("route resolution not implemented"))
}
