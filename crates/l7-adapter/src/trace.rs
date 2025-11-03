use std::borrow::Cow;

use tracing::{span, Level, Span};

#[derive(Clone, Default)]
pub struct AdapterTracer {
    pub component: Cow<'static, str>,
}

impl AdapterTracer {
    pub fn span(&self, tenant: &str, tool: &str) -> Span {
        span!(
            Level::INFO,
            "l7.adapter.http",
            tenant = tenant,
            tool = tool,
            component = %self.component
        )
    }
}
