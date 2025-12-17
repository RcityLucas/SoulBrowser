use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as Base64, Engine as _};
use perceiver_hub::models::MultiModalPerception;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, error, instrument};

use crate::console_fixture::load_console_fixture;
use crate::perception_service::{CookieOverride, PerceptionJob, ViewportConfig};
use crate::server::{rate_limit::RateLimitKind, ServeState};

pub(crate) fn router() -> Router<ServeState> {
    Router::new()
        .route("/api/perceive", post(serve_perceive_handler))
        .route("/api/perceive/metrics", get(perception_metrics_handler))
}

#[derive(Debug, Deserialize)]
struct UiPerceiveRequest {
    url: String,
    structural: Option<bool>,
    visual: Option<bool>,
    semantic: Option<bool>,
    insights: Option<bool>,
    #[serde(default)]
    screenshot: Option<bool>,
    #[serde(default)]
    timeout: Option<u64>,
    mode: Option<String>,
    #[serde(default)]
    viewport: Option<UiViewportConfig>,
    #[serde(default)]
    cookies: Vec<UiCookieParam>,
    #[serde(default)]
    inject_script: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UiViewportConfig {
    width: u32,
    height: u32,
    #[serde(default = "default_device_scale_factor")]
    device_scale_factor: f64,
    #[serde(default)]
    mobile: bool,
    #[serde(default)]
    emulate_touch: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UiCookieParam {
    name: String,
    value: String,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    expires: Option<f64>,
    #[serde(default)]
    http_only: Option<bool>,
    #[serde(default)]
    secure: Option<bool>,
    #[serde(default)]
    same_site: Option<String>,
}

#[derive(Serialize)]
struct UiPerceiveResponse {
    success: bool,
    perception: Option<MultiModalPerception>,
    screenshot_base64: Option<String>,
    stdout: String,
    stderr: String,
    error: Option<String>,
}

#[instrument(
    name = "soul.perception.request",
    skip(state, req),
    fields(client_ip = %client_addr, url = %req.url)
)]
async fn serve_perceive_handler(
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    State(state): State<ServeState>,
    Json(req): Json<UiPerceiveRequest>,
) -> impl IntoResponse {
    if !state
        .rate_limiter
        .allow(&client_addr.ip().to_string(), RateLimitKind::Task)
    {
        let body = UiPerceiveResponse {
            success: false,
            perception: None,
            screenshot_base64: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some("Too many requests".to_string()),
        };
        return (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
    }

    if req.url.trim().is_empty() {
        let body = UiPerceiveResponse {
            success: false,
            perception: None,
            screenshot_base64: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some("URL must not be empty".to_string()),
        };
        return (StatusCode::BAD_REQUEST, Json(body)).into_response();
    }

    if let Ok(Some(fixture)) = load_console_fixture().await {
        let status = if fixture.success {
            StatusCode::OK
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        let body = UiPerceiveResponse {
            success: fixture.success,
            perception: fixture.perception,
            screenshot_base64: fixture.screenshot_base64,
            stdout: fixture.stdout,
            stderr: fixture.stderr,
            error: fixture.error_message,
        };
        return (status, Json(body)).into_response();
    }

    let job = match perception_job_from_request(&req, state.ws_url.clone()) {
        Ok(job) => job,
        Err(message) => {
            let body = UiPerceiveResponse {
                success: false,
                perception: None,
                screenshot_base64: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(message),
            };
            return (StatusCode::BAD_REQUEST, Json(body)).into_response();
        }
    };

    let service = state.perception_service();
    let result = service.perceive(job).await;
    let metrics = service.metrics_snapshot();
    debug!(
        target = "perception_service",
        total_runs = metrics.total_runs,
        shared_hits = metrics.shared_hits,
        shared_misses = metrics.shared_misses,
        shared_failures = metrics.shared_failures,
        ephemeral_runs = metrics.ephemeral_runs,
        failed_runs = metrics.failed_runs,
        avg_duration_ms = metrics.avg_duration_ms,
        "perception metrics"
    );
    match result {
        Ok(output) => {
            let screenshot_base64 = output.screenshot.as_ref().map(|bytes| Base64.encode(bytes));
            let body = UiPerceiveResponse {
                success: true,
                perception: Some(output.perception),
                screenshot_base64,
                stdout: output.log_lines.join("\n"),
                stderr: String::new(),
                error: None,
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(err) => {
            error!(?err, "perception service failed");
            let body = UiPerceiveResponse {
                success: false,
                perception: None,
                screenshot_base64: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(err.to_string()),
            };
            (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
        }
    }
}

#[instrument(name = "soul.perception.metrics", skip(state))]
async fn perception_metrics_handler(State(state): State<ServeState>) -> impl IntoResponse {
    let service = state.perception_service();
    let metrics = service.metrics_snapshot();
    let pooling_cooldown = service.pooling_cooldown_secs();
    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "metrics": {
                "total_runs": metrics.total_runs,
                "shared_hits": metrics.shared_hits,
                "shared_misses": metrics.shared_misses,
                "shared_failures": metrics.shared_failures,
                "ephemeral_runs": metrics.ephemeral_runs,
                "failed_runs": metrics.failed_runs,
                "avg_duration_ms": metrics.avg_duration_ms,
                "pooling": {
                    "enabled": service.pooling_enabled(),
                    "cooldown_secs": pooling_cooldown,
                }
            }
        })),
    )
}

fn perception_job_from_request(
    req: &UiPerceiveRequest,
    ws_url: Option<String>,
) -> Result<PerceptionJob, String> {
    let mode = req.mode.as_deref().unwrap_or("");
    let all_flag = mode.eq_ignore_ascii_case("all");

    let mut enable_structural = req.structural.unwrap_or(false);
    let mut enable_visual = req.visual.unwrap_or(false);
    let mut enable_semantic = req.semantic.unwrap_or(false);

    match mode.to_ascii_lowercase().as_str() {
        "structural" => enable_structural = true,
        "visual" => enable_visual = true,
        "semantic" => enable_semantic = true,
        "all" => {
            enable_structural = true;
            enable_visual = true;
            enable_semantic = true;
        }
        _ => {}
    }

    if !enable_structural && !enable_visual && !enable_semantic {
        enable_structural = true;
        enable_visual = true;
        enable_semantic = true;
    }

    let capture_screenshot = req.screenshot.unwrap_or_else(|| enable_visual || all_flag);
    let enable_insights = req.insights.unwrap_or(all_flag);

    let (viewport, cookies, script, allow_pooling) = build_perception_overrides(req, &req.url)?;

    Ok(PerceptionJob {
        url: req.url.clone(),
        enable_structural,
        enable_visual,
        enable_semantic,
        enable_insights,
        capture_screenshot,
        timeout_secs: req.timeout.unwrap_or(30),
        chrome_path: None,
        ws_url,
        headful: false,
        viewport,
        cookies,
        inject_script: script,
        allow_pooling,
    })
}

fn build_perception_overrides(
    req: &UiPerceiveRequest,
    fallback_url: &str,
) -> Result<
    (
        Option<ViewportConfig>,
        Vec<CookieOverride>,
        Option<String>,
        bool,
    ),
    String,
> {
    let mut allow_pooling = true;
    let viewport = if let Some(config) = req.viewport.as_ref() {
        if config.width == 0 || config.height == 0 {
            return Err("viewport width/height must be positive".to_string());
        }

        allow_pooling = false;
        Some(ViewportConfig {
            width: config.width,
            height: config.height,
            device_scale_factor: config.device_scale_factor.max(0.1),
            mobile: config.mobile,
            emulate_touch: config.emulate_touch.unwrap_or(config.mobile),
        })
    } else {
        None
    };

    let mut cookies = Vec::new();
    for raw in &req.cookies {
        let cookie = build_cookie_override(raw, fallback_url)?;
        allow_pooling = false;
        cookies.push(cookie);
    }

    let script = req.inject_script.as_ref().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    if script.is_some() {
        allow_pooling = false;
    }

    Ok((viewport, cookies, script, allow_pooling))
}

fn build_cookie_override(
    raw: &UiCookieParam,
    fallback_url: &str,
) -> Result<CookieOverride, String> {
    let name = raw.name.trim();
    if name.is_empty() {
        return Err("cookie name must not be empty".to_string());
    }

    let value = raw.value.clone();
    let domain = raw.domain.as_ref().and_then(trimmed_non_empty);
    let path = raw.path.as_ref().and_then(trimmed_non_empty);
    let url = raw
        .url
        .as_ref()
        .and_then(trimmed_non_empty)
        .or_else(|| (domain.is_none()).then(|| fallback_url.to_string()));

    let same_site = match raw.same_site.as_ref().and_then(trimmed_non_empty) {
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "lax" => Some("Lax".to_string()),
            "strict" => Some("Strict".to_string()),
            "none" => Some("None".to_string()),
            other => {
                return Err(format!(
                    "unsupported same_site '{}'; use lax|strict|none",
                    other
                ));
            }
        },
        None => None,
    };

    Ok(CookieOverride {
        name: name.to_string(),
        value,
        domain,
        path,
        url,
        expires: raw.expires,
        http_only: raw.http_only,
        secure: raw.secure,
        same_site,
    })
}

fn trimmed_non_empty(value: &String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

const fn default_device_scale_factor() -> f64 {
    1.0
}
