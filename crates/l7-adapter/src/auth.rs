use axum::http::{header, HeaderMap, HeaderValue};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use time::OffsetDateTime;
use tonic::metadata::MetadataMap;

use crate::policy::TenantPolicy;

const SIGNATURE_HEADER: &str = "x-signature";
const SIGNATURE_TS_HEADER: &str = "x-signature-timestamp";
const SIGNATURE_TOLERANCE: i64 = 300; // seconds

pub fn verify_http(
    headers: &HeaderMap,
    tenant: &TenantPolicy,
    payload_json: &str,
) -> Result<(), &'static str> {
    if !tenant.api_keys.is_empty() {
        ensure_api_key(
            headers.get(header::AUTHORIZATION),
            headers.get("x-tenant-token"),
            tenant,
        )?;
    }

    if !tenant.hmac_secrets.is_empty() {
        ensure_hmac(
            headers.get(SIGNATURE_HEADER),
            headers.get(SIGNATURE_TS_HEADER),
            tenant,
            payload_json,
        )?;
    }

    Ok(())
}

pub fn verify_grpc(
    metadata: &MetadataMap,
    tenant: &TenantPolicy,
    payload_json: &str,
) -> Result<(), &'static str> {
    if !tenant.api_keys.is_empty() {
        let auth_header = metadata
            .get(header::AUTHORIZATION.as_str())
            .and_then(|value| value.to_str().ok());
        let token_header = metadata
            .get("x-tenant-token")
            .and_then(|value| value.to_str().ok());
        ensure_api_key_ascii(auth_header, token_header, tenant)?;
    }

    if !tenant.hmac_secrets.is_empty() {
        let signature = metadata
            .get(SIGNATURE_HEADER)
            .and_then(|value| value.to_str().ok());
        let timestamp = metadata
            .get(SIGNATURE_TS_HEADER)
            .and_then(|value| value.to_str().ok());
        ensure_hmac_ascii(signature, timestamp, tenant, payload_json)?;
    }

    Ok(())
}

fn ensure_api_key(
    authorization: Option<&HeaderValue>,
    token_header: Option<&HeaderValue>,
    tenant: &TenantPolicy,
) -> Result<(), &'static str> {
    let mut valid = false;
    if let Some(value) = authorization.and_then(|value| value.to_str().ok()) {
        if let Some(token) = value.strip_prefix("Bearer ") {
            if contains_api_key(token, tenant) {
                valid = true;
            }
        }
    }

    if !valid {
        if let Some(value) = token_header.and_then(|value| value.to_str().ok()) {
            if contains_api_key(value, tenant) {
                valid = true;
            }
        }
    }

    if valid {
        Ok(())
    } else {
        Err("invalid tenant token")
    }
}

fn ensure_api_key_ascii(
    authorization: Option<&str>,
    token_header: Option<&str>,
    tenant: &TenantPolicy,
) -> Result<(), &'static str> {
    let mut valid = false;
    if let Some(value) = authorization {
        if let Some(token) = value.strip_prefix("Bearer ") {
            if contains_api_key(token, tenant) {
                valid = true;
            }
        }
    }

    if !valid {
        if let Some(value) = token_header {
            if contains_api_key(value, tenant) {
                valid = true;
            }
        }
    }

    if valid {
        Ok(())
    } else {
        Err("invalid tenant token")
    }
}

fn contains_api_key(candidate: &str, tenant: &TenantPolicy) -> bool {
    tenant
        .api_keys
        .iter()
        .any(|key| key.as_bytes().ct_eq(candidate.as_bytes()).unwrap_u8() == 1)
}

type HmacSha256 = Hmac<Sha256>;

fn ensure_hmac(
    signature: Option<&HeaderValue>,
    timestamp: Option<&HeaderValue>,
    tenant: &TenantPolicy,
    payload_json: &str,
) -> Result<(), &'static str> {
    let signature = signature
        .and_then(|value| value.to_str().ok())
        .ok_or("missing signature header")?;
    let timestamp = timestamp
        .and_then(|value| value.to_str().ok())
        .ok_or("missing signature timestamp")?;

    ensure_hmac_common(signature, timestamp, tenant, payload_json)
}

fn ensure_hmac_ascii(
    signature: Option<&str>,
    timestamp: Option<&str>,
    tenant: &TenantPolicy,
    payload_json: &str,
) -> Result<(), &'static str> {
    let signature = signature.ok_or("missing signature header")?;
    let timestamp = timestamp.ok_or("missing signature timestamp")?;
    ensure_hmac_common(signature, timestamp, tenant, payload_json)
}

fn ensure_hmac_common(
    signature: &str,
    timestamp: &str,
    tenant: &TenantPolicy,
    payload_json: &str,
) -> Result<(), &'static str> {
    let timestamp_value = timestamp
        .parse::<i64>()
        .map_err(|_| "invalid signature timestamp")?;
    let now = OffsetDateTime::now_utc().unix_timestamp();
    if (now - timestamp_value).abs() > SIGNATURE_TOLERANCE {
        return Err("signature timestamp expired");
    }

    let message = format!("{}:{}", timestamp, payload_json);
    let provided = hex::decode(signature).map_err(|_| "invalid signature encoding")?;

    for secret in &tenant.hmac_secrets {
        if verify_hmac(secret, &message, &provided) {
            return Ok(());
        }
    }

    Err("signature verification failed")
}

fn verify_hmac(secret: &str, message: &str, provided: &[u8]) -> bool {
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    mac.update(message.as_bytes());
    let expected = mac.finalize().into_bytes();
    expected.ct_eq(provided).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use serde::{Deserialize, Serialize};
    use tonic::metadata::MetadataMap;

    #[derive(Serialize, Deserialize)]
    struct TestPayload {
        value: String,
    }

    fn tenant_with_keys() -> TenantPolicy {
        TenantPolicy {
            id: "tenant".into(),
            allow_tools: Vec::new(),
            allow_flows: Vec::new(),
            read_only: Vec::new(),
            rate_limit_rps: 1,
            rate_burst: 1,
            concurrency_max: 1,
            timeout_ms_tool: 1_000,
            timeout_ms_flow: 1_000,
            timeout_ms_read: 1_000,
            idempotency_window_sec: 60,
            allow_cold_export: false,
            exports_max_lines: 1_000,
            authz_scopes: Vec::new(),
            api_keys: vec!["secret-token".into()],
            hmac_secrets: Vec::new(),
        }
    }

    #[test]
    fn verify_api_key_header() {
        let tenant = tenant_with_keys();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret-token"),
        );
        let payload_json = serde_json::to_string(&TestPayload { value: "x".into() }).unwrap();
        verify_http(&headers, &tenant, &payload_json).unwrap();
    }

    #[test]
    fn verify_api_key_token_header() {
        let tenant = tenant_with_keys();
        let mut headers = HeaderMap::new();
        headers.insert("x-tenant-token", HeaderValue::from_static("secret-token"));
        let payload_json = serde_json::to_string(&TestPayload { value: "x".into() }).unwrap();
        verify_http(&headers, &tenant, &payload_json).unwrap();
    }

    #[test]
    fn verify_hmac_signature() {
        let mut tenant = tenant_with_keys();
        tenant.api_keys.clear();
        tenant.hmac_secrets.push("shared".into());

        let mut headers = HeaderMap::new();
        let timestamp = OffsetDateTime::now_utc().unix_timestamp().to_string();
        let payload = TestPayload { value: "x".into() };
        let payload_json = serde_json::to_string(&payload).unwrap();
        let message = format!("{}:{}", timestamp, payload_json);
        let mut mac = HmacSha256::new_from_slice(b"shared").unwrap();
        mac.update(message.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());
        headers.insert(
            SIGNATURE_TS_HEADER,
            HeaderValue::from_str(&timestamp).unwrap(),
        );
        headers.insert(SIGNATURE_HEADER, HeaderValue::from_str(&signature).unwrap());

        verify_http(&headers, &tenant, &payload_json).unwrap();
    }

    #[test]
    fn reject_expired_signature() {
        let mut tenant = tenant_with_keys();
        tenant.api_keys.clear();
        tenant.hmac_secrets.push("shared".into());

        let mut headers = HeaderMap::new();
        let timestamp = (OffsetDateTime::now_utc().unix_timestamp() - 1_000).to_string();
        let payload = TestPayload { value: "x".into() };
        let payload_json = serde_json::to_string(&payload).unwrap();
        let message = format!("{}:{}", timestamp, payload_json);
        let mut mac = HmacSha256::new_from_slice(b"shared").unwrap();
        mac.update(message.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());
        headers.insert(
            SIGNATURE_TS_HEADER,
            HeaderValue::from_str(&timestamp).unwrap(),
        );
        headers.insert(SIGNATURE_HEADER, HeaderValue::from_str(&signature).unwrap());

        assert!(verify_http(&headers, &tenant, &payload_json).is_err());
    }

    #[test]
    fn verify_grpc_with_bearer() {
        let tenant = tenant_with_keys();
        let mut metadata = MetadataMap::new();
        metadata.insert("authorization", "Bearer secret-token".parse().unwrap());
        let payload = TestPayload { value: "x".into() };
        let payload_json = serde_json::to_string(&payload).unwrap();
        verify_grpc(&metadata, &tenant, &payload_json).unwrap();
    }

    #[test]
    fn verify_grpc_with_signature() {
        let mut tenant = tenant_with_keys();
        tenant.api_keys.clear();
        tenant.hmac_secrets.push("shared".into());

        let mut metadata = MetadataMap::new();
        let timestamp = OffsetDateTime::now_utc().unix_timestamp().to_string();
        let payload = TestPayload { value: "x".into() };
        let payload_json = serde_json::to_string(&payload).unwrap();
        let message = format!("{}:{}", timestamp, payload_json);
        let mut mac = HmacSha256::new_from_slice(b"shared").unwrap();
        mac.update(message.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());
        metadata.insert(SIGNATURE_TS_HEADER, timestamp.parse().unwrap());
        metadata.insert(SIGNATURE_HEADER, signature.parse().unwrap());

        verify_grpc(&metadata, &tenant, &payload_json).unwrap();
    }
}
