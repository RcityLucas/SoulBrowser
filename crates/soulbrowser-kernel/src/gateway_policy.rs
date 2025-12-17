use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use url::form_urlencoded;

#[derive(Clone, Debug, Default)]
pub struct GatewayPolicy {
    pub allowed_tokens: Vec<String>,
    pub ip_whitelist: Vec<IpAddr>,
}

impl GatewayPolicy {
    pub fn from_tokens_and_ips(tokens: Vec<String>, ips: Vec<IpAddr>) -> Self {
        Self {
            allowed_tokens: tokens,
            ip_whitelist: ips,
        }
    }

    pub fn allows(&self, token: &str, ip: &IpAddr) -> bool {
        let ip_allowed =
            self.ip_whitelist.is_empty() || self.ip_whitelist.iter().any(|item| item == ip);
        let token_allowed = self.allowed_tokens.is_empty()
            || self.allowed_tokens.iter().any(|allowed| allowed == token);
        ip_allowed && token_allowed
    }
}

pub async fn gateway_auth_middleware(
    State(policy): State<Arc<GatewayPolicy>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let ip = client_addr.ip();
    if !policy.ip_whitelist.is_empty() && !policy.ip_whitelist.iter().any(|allowed| *allowed == ip)
    {
        return Err(StatusCode::FORBIDDEN);
    }

    let provided_token = extract_token(&req);
    match provided_token {
        Some(token)
            if policy.allowed_tokens.is_empty() || policy.allowed_tokens.contains(&token) =>
        {
            Ok(next.run(req).await)
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

pub async fn gateway_ip_middleware(
    State(policy): State<Arc<GatewayPolicy>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let ip = client_addr.ip();
    if policy.ip_whitelist.is_empty() || policy.ip_whitelist.iter().any(|allowed| *allowed == ip) {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

fn extract_token(req: &Request<Body>) -> Option<String> {
    let headers = req.headers();
    if let Some(value) = headers.get("x-soulbrowser-token") {
        if let Ok(raw) = value.to_str() {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    if let Some(value) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(raw) = value.to_str() {
            if let Some(token) = raw.strip_prefix("Bearer ") {
                let trimmed = token.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    if let Some(query) = req.uri().query() {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            if key == "token" || key == "auth_token" {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    None
}
