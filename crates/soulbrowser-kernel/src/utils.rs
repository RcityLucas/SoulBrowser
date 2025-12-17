use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use cdp_adapter::{events::RawEvent, ids::PageId as AdapterPageId, CdpAdapter, EventStream};
use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::timeout;
use tracing::warn;

pub fn ensure_real_chrome_enabled() -> Result<()> {
    let flag = env::var("SOULBROWSER_USE_REAL_CHROME")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(flag.as_str(), "1" | "true" | "yes" | "on") {
        Ok(())
    } else {
        bail!(
            "Set SOULBROWSER_USE_REAL_CHROME=1 to run this command against a real Chrome/Chromium binary"
        );
    }
}

pub fn build_exec_route(adapter: &Arc<CdpAdapter>, page_id: AdapterPageId) -> Result<ExecRoute> {
    let context = adapter
        .registry()
        .iter()
        .into_iter()
        .find(|(pid, _)| pid == &page_id)
        .map(|(_, ctx)| ctx)
        .ok_or_else(|| anyhow!("no registry context available for page {:?}", page_id))?;

    let session = SessionId(context.session_id.0.to_string());
    let page = PageId(page_id.0.to_string());
    let frame_key = context.target_id.clone().unwrap_or_else(|| page.0.clone());
    let frame = FrameId(frame_key);

    Ok(ExecRoute::new(session, page, frame))
}

pub async fn wait_for_page_ready(
    adapter: Arc<CdpAdapter>,
    rx: &mut EventStream,
    wait_limit: Duration,
    log: &mut Vec<String>,
) -> Result<AdapterPageId> {
    let deadline = Instant::now() + wait_limit;
    loop {
        if let Some((page_id, _ctx)) = adapter
            .registry()
            .iter()
            .into_iter()
            .find(|(_, ctx)| ctx.cdp_session.is_some())
        {
            return Ok(page_id);
        }

        if Instant::now() >= deadline {
            let preview = log.iter().take(16).cloned().collect::<Vec<_>>().join(" | ");
            bail!(
                "Timed out waiting for Chrome target/session. Recent events: {}",
                preview
            );
        }

        match timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Ok(event)) => {
                log.push(describe_raw_event(&event));
            }
            Ok(Err(RecvError::Lagged(skipped))) => {
                warn!(skipped, "Demo event stream lagged; skipping older events");
            }
            Ok(Err(RecvError::Closed)) => bail!("CDP adapter event stream closed unexpectedly"),
            Err(_) => {}
        }
    }
}

pub async fn collect_events(
    rx: &mut EventStream,
    duration: Duration,
    log: &mut Vec<String>,
) -> Result<()> {
    if duration.is_zero() {
        return Ok(());
    }

    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_default();
        if remaining.is_zero() {
            break;
        }
        let slice = remaining.min(Duration::from_millis(500));

        match timeout(slice, rx.recv()).await {
            Ok(Ok(event)) => log.push(describe_raw_event(&event)),
            Ok(Err(RecvError::Lagged(skipped))) => {
                warn!(skipped, "Demo event stream lagged; skipping older events");
            }
            Ok(Err(RecvError::Closed)) => {
                warn!("Demo event stream closed");
                break;
            }
            Err(_) => {}
        }
    }

    Ok(())
}

fn describe_raw_event(event: &RawEvent) -> String {
    match event {
        RawEvent::PageLifecycle {
            page, frame, phase, ..
        } => {
            let frame_str = frame.map(|f| format!(" frame={:?}", f)).unwrap_or_default();
            format!("page {:?} phase={}{}", page, phase, frame_str)
        }
        RawEvent::PageNavigated { page, url, .. } => {
            format!("page {:?} navigated -> {}", page, url)
        }
        RawEvent::NetworkSummary {
            page,
            req,
            res2xx,
            res4xx,
            res5xx,
            inflight,
            quiet,
            since_last_activity_ms,
            ..
        } => format!(
            "network {:?} req={} 2xx={} 4xx={} 5xx={} inflight={} quiet={} idle={}ms",
            page, req, res2xx, res4xx, res5xx, inflight, quiet, since_last_activity_ms
        ),
        RawEvent::NetworkActivity { page, signal } => {
            format!("network-activity {:?} signal={:?}", page, signal)
        }
        RawEvent::Error { message, .. } => format!("adapter-error: {message}"),
    }
}
