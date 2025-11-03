# L7 · Interfaces & Ecosystem Overview

L7 exposes a pair of optional surfaces that stay disabled until explicitly whitelisted via policy:

- **Adapter Gateway (`crates/l7-adapter`)** – HTTP first (Axum router) with stubs for gRPC/MCP. Policies gate access per tenant (tool allow-lists, rate/concurrency caps, privacy profile bindings). Requests are filtered before reaching L1/L5 and are ready to forward trace metadata into L6 observe.
- **Plugin Sandbox (`crates/l7-plugin`)** – Manifest-driven registry with kill-switch and trust-level controls. The sandbox host is currently a stub (WASM executor to follow), but manifests, policy handles, and registry wiring are in place so governance flows can be validated end-to-end.

### Current behavior

- `/healthz` is always live; `/v1/tools/run` responds only when the adapter is enabled and a tenant is authorized. The dispatcher/read-only ports default to `Noop` implementations so downstream wiring can be added incrementally.
- Plugins load through the registry, which immediately blocks entries that match policy kill-switch patterns and mirrors the global enable flag.

### Next steps

1. Fill in auth/guard implementations (OAuth2, rate limiting, idempotency) and wire the dispatcher to real L1/L5 ports.
2. Implement the WASM sandbox executor and host hooks, plus observation/export integrations.
3. Extend integration coverage for gRPC/MCP once the HTTP façade stabilises.

### Recent progress (2025-??)

- HTTP/gRPC adapters now apply API-key/Bearer/HMAC authentication per tenant policy, with shared logic in `crates/l7-adapter/src/auth.rs`.
- Both transports support idempotent tool execution through `IdempotencyStore`, returning cached outcomes when clients reuse keys within the policy TTL.
- Added unit tests validating token enforcement, signature verification, and idempotency behaviour for HTTP/gRPC handlers.
- WebDriver bridge remains mapped to scheduler but still uses token-based gating; full OAuth/OIDC integration and MCP endpoints are pending.
- Plugin sandbox runtime is still a stub – execution, hook policy enforcement, and MCP integration are outstanding.

**Next steps**
1. Implement plugin runtime wiring (install/execute endpoints) with tenant-aware policy checks and metrics/audit sinks.
2. Extend MCP/gRPC surfaces to invoke plugins, mirroring HTTP functionality.
3. Build end-to-end tests covering external trigger → scheduler dispatch → tool/plug-in execution → observability events.
4. Conduct security/privacy review once authentication flows and sandbox execution are in place.
