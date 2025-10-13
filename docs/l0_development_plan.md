# L0 Runtime & Adapters Development Plan

This plan tracks the implementation work needed to deliver the L0 “运行与适配” layer for SoulBrowser. It mirrors the architecture described in the L0 design docs and breaks the effort into milestones that can be executed iteratively.

## Current Scaffolding

- `crates/cdp-adapter/` – command/event surface stubs and error/types for the forthcoming CDP integration.
- `crates/permissions-broker/` – policy/decision types plus trait placeholders for permission management.
- `crates/network-tap-light/` – summary/snapshot payloads and tap skeleton.
- `crates/stealth/` – profile, tempo, and captcha contract definitions.
- `crates/extensions-bridge/` – request/response protocol and channel interfaces.

 Recent progress:
 - TTL-aware policy decisions with crate tests (`cargo test -p permissions-broker`).
 - In-memory network tap state registry with crate tests (`cargo test -p network-tap-light`).
 - Stealth runtime now keeps an in-memory profile catalog and applied profile cache, with smoke tests (`cargo test -p stealth`).
 - CDP adapter exposes a pluggable transport with an event-loop smoke test (`cargo test -p cdp-adapter`).
 - Extensions bridge now maintains an in-memory channel registry with smoke tests (`cargo test -p extensions-bridge`).

## Milestones

### M1 · CDP Adapter Core (3–4 weeks)

- **Environment prep**: choose the Chromium/CDP client crate, decide how to distribute the browser binary, and wire launch configuration.
- **Crate bootstrap**: create `crates/cdp-adapter/`, define the adapter facade, event bus contract, registry, and error model.
- **Command surface**: implement the 8 minimal capabilities (`navigate`, `query`, `click`, `type_text`, `select`, `wait_basic`, `screenshot`, `dom_snapshot`/`ax_snapshot`, `set_network_tap`).
- **Runtime loops**: ingest CDP events, manage health/heartbeat, reconnection, and metrics/telemetry hooks.
- **Tests**: smoke tests against a local headless Chromium build; configure CI to run headless browsers.

### M2 · L0 Satellite Modules (4–5 weeks, with overlap)

1. **permissions-broker (≈2 weeks)**
   - Parse policy templates, maintain per-origin caches/TTL, expose audit events, and hook into the CDP permissions port.
2. **network-tap (light) (≈1.5 weeks)**
   - Build the aggregation loop, quiet detection, filtering controls, and `RawEvent::NetworkSummary` publishing + snapshots.
3. **stealth-fingerprint & captcha-channel (≈2 weeks)**
   - Implement profile/tempo policy parsing, UA/timezone/viewport injection, tempo guidance API, captcha detection and decision scaffolding.
4. **extensions-bridge (≈2 weeks)**
   - Add extension allowlist/policy checks, channel handshake for tab/background scopes, JSON request/response protocol, permission coordination, and graceful degradation.

### M3 · CLI Integration & Validation (2–3 weeks)

- Integrate the new crates into the CLI/automation flow with feature flags for gradual rollout.
- Extend configuration and context wiring so commands can toggle L0 features.
- Add end-to-end scenarios that cover basic browsing, permission denial, network quiet gates, captcha detection, and extension calls.
- Expand observability (tracing, metrics, audits) and documentation (operations guide, deployment notes).

## Cross-Cutting Tasks

- **CI/CD**: create headless-browser pipelines, cache browser binaries, and enable matrix runs (Linux/macOS/Windows if feasible).
- **Documentation**: keep crate-level READMEs/ADRs up to date, document configuration knobs and troubleshooting steps.
- **Security & Compliance**: review permission templates, fingerprint policies, and extension allowlists with the security team before enabling in production.

This plan should be updated as milestones land or scope changes. Treat it as the authoritative checklist for L0 delivery.
