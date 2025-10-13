# L1 Unified Kernel · Development Plan (2025-01)

> Scope: deliver the four L1 submodules described in the design docs — Registry, Scheduler, State Center, Policy Center — and wire them into the SoulBrowser CLI. This plan converts those documents into an executable roadmap.

## 0. Guiding Outcomes
- Deterministic routing + lifecycle management for sessions/pages/frames (Registry).
- Fair, cancellable ToolCall execution with bounded concurrency (Scheduler).
- Short-term factual memory + minimal replay for diagnostics (State Center).
- Centralised, auditable policy/feature control with hot overrides (Policy Center).
- Observatory signals (events/metrics) available everywhere and consumable by L6.

## 1. Phase Breakdown & Deliverables

### Phase 0 · Scaffolding & Common Foundations (0.5 week)
- Create crates: `crates/registry`, `crates/scheduler`, `crates/state-center`, `crates/policy-center`.
- Expose shared primitives crate (`crates/core-types` or interim module) for ExecRoute, ToolCall, SoulError, RawEvent.
- Wire crates into `Cargo.toml`, add minimal `lib.rs`, ensure `cargo check` passes.
- Set up integration test harness (tokio + `tests/l1_smoke.rs`) and event-bus mock stubs.
- Exit criteria: workspace builds; CI job added for `cargo fmt`, `cargo clippy`, `cargo test --all`.

### Phase 1 · Session·Tab·Frame Registry (1.5 weeks)
- Implement data model + storage (dashmap + RwLock contexts, reverse mappings).
- Build ingestion pipeline from EventBus (mock) for target/frame/lifecycle events.
- Surface public async trait `Registry` and provide concrete `RegistryImpl`.
- Implement routing resolution, default fallback, error mapping, page health proxy.
- Unit tests covering lifecycle transitions, resync, routing preferences.
- Exit criteria: `registry` crate publishes events, handles resync, passes 90% coverage on core paths.

### Phase 2 · Dispatcher & Scheduler (2 weeks)
- Implement ToolCall validation, dedup, priority queues (WRR/DRR lanes).
- Integrate with Registry for route resolution + sticky mutex keys.
- Add global semaphore, per-task counters, cancellation tokens, retry logic.
- Provide ToolRunner trait + registration API for L5 tools (mock runner for tests).
- Emit DISPATCH_* events + metrics; cover ServerBusy, RouteStale recovery, lightning preemption.
- Exit criteria: concurrency invariants proven via async tests; schedule timeline recorded in State Center stub.

### Phase 3 · State Center (1 week)
- Implement unified `StateEvent`, ring buffers (global/session/page/task).
- Build append pipeline with redact/drop policy + ingestion channel.
- Provide history queries + minimal replay builder; integrate with Scheduler + Registry emitted events.
- Add metrics on ingest/dropped/latency.
- Exit criteria: history/replay endpoints return deterministic data in tests; backpressure policy verified.

_Progress note (2025-01): dispatch success/failure 与基础 registry 生命周期事件已写入内存 State Center，可通过 CLI `info`/`scheduler` 查询（并支持取消未执行 action）。Policy Center 支持默认快照与运行时 override（`soulbrowser policy show|override`）；Metrics export 与 Minimal Replay 仍待完成。_

### Phase 4 · Policy & Feature Flags (1 week)
- Implement PolicySnapshot + cascade loader (builtin → file → env/cli → overrides).
- Provide runtime overrides with TTL + provenance tracking + sticky guard.
- Fan-out module views to Registry/Scheduler/State Center mocks; enforce “stricter wins” rules.
- Emit POLICY_* / FEATURE_* events; integrate with metrics.
- Exit criteria: unit tests cover merge precedence, dependency validation, TTL rollback; CLI command `soulbrowser policy reload` (placeholder).

### Phase 5 · Integration & Hardening (1.5 weeks)
- Wire Scheduler + Registry into CLI execution path behind `--enable-unified-kernel` flag.
- Connect State Center to EventBus + CLI diagnostics command.
- Hook Policy Center into startup (load + watch) and runtime override API.
- Add end-to-end tests (mock L0) for session open → navigate → ToolCall → observation flow.
- Instrument metrics exporters; ensure State Center events align with docs.
- Exit criteria: CLI smoke test passes; metrics visible; plan review sign-off.

## 2. Cross-Cutting Tasks
- **Event Bus**: define lightweight trait + in-memory bus for local tests now, plan for swap with real bus in L2.
- **Errors**: implement `SoulError` variants for Registry/Scheduler/Policy; ensure mapping is consistent.
- **Metrics**: adopt `metrics` crate; configure sinks in CLI.
- **Docs**: update README + new docs per phase (e.g. `docs/l1_registry.md`).
- **Automation**: add `just`/`make` targets or cargo tasks for lint/test.

## 3. Dependencies & Risks
- Need stable interfaces from L0 crates (cdp-adapter, network-tap-light, permissions-broker) — provide adapter traits + mocks early.
- Missing core shared types: resolve by introducing `crates/core-types` or embedding interim module, refactor later.
- Concurrency testing may be flaky: invest in deterministic tokio test utilities + manual seeds.
- Policy file location / format: confirm with DevOps; include sample `config/policy.yaml`.

## 4. Validation Strategy
- Unit tests per crate (tokio `#[tokio::test(flavor = "multi_thread")]`).
- Integration tests in `tests/` using mock L0 to simulate event flow.
- Benchmark micro-tests for routing and scheduling (target: <1 ms P95 as documented).
- Manual CLI smoke script executed at Phase 5 sign-off.
- Metrics/assertions verified via test sink.

## 5. Timeline Snapshot (6.5 weeks total)
- Week 1: Phase 0 + start Phase 1.
- Week 2: Finish Phase 1; begin Phase 2.
- Week 3: Complete Scheduler core; start cancellation/preemption tests.
- Week 4: Phase 2 hardening + begin State Center.
- Week 5: Finish State Center; deliver Policy Center core.
- Week 6: Policy hardening + integration wiring.
- Week 6.5: Buffer for hardening, docs, release review.

## 6. Immediate Next Steps
1. Execute **Phase 0** tasks (crate scaffolding, shared types, CI tweaks).
2. Draft mock EventBus + shared error crate; confirm with team (create ADR if needed).
3. Begin Registry implementation per Phase 1 once scaffolding green.

_This plan supersedes the earlier draft; keep it updated at the end of each phase._
