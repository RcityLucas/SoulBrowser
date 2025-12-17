# L6 · Governance & Observability

## Metrics & Tracing Quickstart

### Runtime Toggle
The observability layer is controlled through `ObsPolicyView` (hot-reloaded via Policy Center). Defaults:
- metrics/tracing enabled
- Prometheus bind `0.0.0.0:9090`
- Latency slow threshold `1500ms`
- PII guard on (host-only origin)

### Recording Metrics
`BrowserToolExecutor` records per-tool metrics automatically:
- `soul.l5.tool.invocations{tool="click",success="true"}`
- `soul.l5.tool.latency_ms{tool="click",success="true"}`

Manual instrumentation:
```rust
use l6_observe::{metrics, guard::LabelMap};
let mut labels = LabelMap::new();
labels.insert("component".into(), "scheduler".into());
metrics::inc("soul.l1.scheduler.invocations", labels);
```

### Tracing
Tracing is initialized once via `obs_tracing::init_tracing()` and a span helper is exposed.
Within an executor:
```rust
let span = obs_tracing::tool_span("navigate-to-url");
let _guard = span.enter();
// ... work ...
obs_tracing::observe_latency(&span, latency_ms);
```

### Scheduler Instrumentation
`ToolManagerExecutorAdapter` now emits scheduler dispatch metrics and spans:
- `soul.l1.scheduler.dispatches{tool="click",success="true"}`
- `soul.l1.scheduler.dispatch_latency_ms{tool="click",success="true"}`

All dispatch paths share the same `tool` span, making the end-to-end trace from L1 scheduler → L5 tool visible in tracing backends.

### Headless/Headful Validation
Real-browser tests (`tests/l5_real_adapter.rs`) cover all 12 tools in both headless & headful Chrome sessions.
```
export SOULBROWSER_USE_REAL_CHROME=1
export SOULBROWSER_DISABLE_SANDBOX=1
export SOULBROWSER_CHROME=/usr/bin/google-chrome
cargo test --test l5_real_adapter -- --test-threads=1

# Headful debugging (requires GUI)
export SOUL_HEADLESS=false
cargo test --test l5_real_adapter -- --test-threads=1
```

The tests confirm metrics/tracing hooks execute without breaking tool flows.
