use crate::policy::current_policy;
use once_cell::sync::OnceCell;
use tracing::{span, Level, Span};
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

static INIT: OnceCell<()> = OnceCell::new();

pub fn init_tracing() {
    INIT.get_or_init(|| {
        let policy = current_policy();
        if !policy.enable_tracing {
            return;
        }
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let fmt_layer = fmt::layer()
            .with_ansi(false)
            .with_target(false)
            .with_thread_ids(true);
        let subscriber = Registry::default().with(filter).with(fmt_layer);
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}

pub fn tool_span(name: &str) -> Span {
    span!(Level::INFO, "tool", tool = %name)
}

pub fn observe_latency(span: &Span, latency_ms: u64) {
    span.record("latency_ms", &latency_ms);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_tracing() {
        init_tracing();
        let span = tool_span("unit_test");
        observe_latency(&span, 123);
        span.in_scope(|| tracing::info!("within span"));
    }
}
