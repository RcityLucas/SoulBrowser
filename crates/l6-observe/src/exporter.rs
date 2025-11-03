use crate::metrics::{ensure_metrics, render_prometheus};
use crate::policy::current_policy;
use axum::{routing::get, Router};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::sync::OnceCell as TokioOnceCell;
use tokio::task::JoinHandle;

static PROM_SERVER: TokioOnceCell<JoinHandle<()>> = TokioOnceCell::const_new();

pub fn ensure_prometheus() {
    let policy = current_policy();
    if !policy.prom_enable {
        return;
    }
    ensure_metrics();

    if PROM_SERVER.get().is_some() {
        return;
    }

    let bind: SocketAddr = policy
        .prom_bind
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 9090)));

    let handle = tokio::spawn(async move {
        match TcpListener::bind(bind).await {
            Ok(listener) => {
                let router = Router::new().route("/metrics", get(scrape_handler));
                if let Err(err) = axum::serve(listener, router.into_make_service()).await {
                    tracing::warn!(%err, "prometheus server exited unexpectedly");
                }
            }
            Err(err) => tracing::warn!(%err, "failed to bind prometheus listener"),
        }
    });

    let _ = PROM_SERVER.set(handle);
}

async fn scrape_handler() -> String {
    render_prometheus()
}
