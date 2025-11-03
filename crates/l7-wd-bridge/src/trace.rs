use tracing::{span, Level, Span};

#[derive(Clone, Default)]
pub struct BridgeTracer;

impl BridgeTracer {
    pub fn span(&self, command: &str) -> Span {
        span!(Level::INFO, "l7.webdriver", command = command)
    }
}
