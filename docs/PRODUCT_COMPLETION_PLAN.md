# SoulBrowser 1.0 · Product Completion Plan

**Document date**: 2025-10-21  
**Maintainer**: SoulBrowser Core Team

## 1. Current Delivery Snapshot

| Layer | Status | Key Evidence |
|-------|--------|--------------|
| L0 Runtime & Adapters | ~70% complete | `crates/cdp-adapter` and peers feature-complete; pending full integration tests and recovery tuning.
| L1 Unified Kernel | ~80% complete | Registry/scheduler/state/policy/event-bus shipped; observability + replay polish outstanding.
| L2 Layered Perception | Production-ready | Structural/visual/semantic perceivers & hub fully implemented and tested.
| L3 Intelligent Action | Functional, needs hardening | Core primitives + gates live; flow parallelism and resilience improvements pending.
| L4 Elastic Persistence | Baseline available | Event store/snapshot store/recipes ready; streaming export & governance backlog.
| L5 Tool Layer | Production-ready | 12 tools exercised against real Chrome in headless/headful modes.
| L6 Governance & Observability | Partially enabled | Metrics/tracing hooks wired; needs export surface, alerting, privacy toggles.
| L7 Interfaces & Ecosystem | Scaffolding only | HTTP/gRPC adapter + plugin sandbox stubs present; auth & execution wiring TBD.

## 2. Delivery Objectives

1. **Stabilize real-browser execution** so that end-to-end runs are reliable under load and recover from failures automatically.
2. **Expose actionable observability** (metrics, replay timelines, privacy controls) to support production operations.
3. **Harden persistence and governance** to guarantee evidence durability, auditability, and policy compliance.
4. **Graduate external interfaces** behind controlled rollout once internal QA and governance gates pass.
5. **Ship a 1.0 release candidate** with documented features, test coverage, and upgrade guidance.

## 3. Phase Plan & Milestones

### Phase A · Runtime Hardening (Week 1-2)
- Finalize L0 CDP adapter integration tests (headless/headful, reconnection, concurrency) and error taxonomy.
- Complete permissions broker, network tap, stealth, and extensions bridge polish items from `L0_ACTUAL_PROGRESS.md`.
- Produce "real Chrome" regression suite and CI hooks.
- Deliver updated runtime troubleshooting guide.

**Exit criteria**: 95% pass rate across L0 integration suite; documented recovery procedures; updated `L0_ACTUAL_PROGRESS.md` with >90% completion.

### Phase B · Kernel Observability & Replay (Week 2-3)
- Implement Prometheus metrics export and `/metrics` endpoint (scheduler, registry, CDP stats).
- Complete minimal replay pipeline in state center with CLI export/view commands.
- Wire observability toggles (slow-call alerts, privacy guard config) through Policy Center.
- Add integration tests for cancellation, replay export, and metrics endpoint smoke check.

**Exit criteria**: Metrics endpoint available with validated counters/histograms; replay export CLI delivers usable timelines; L1 docs updated to 95% completion.

### Phase C · Persistence & Governance (Week 3-4)
- Introduce streaming export + alerting in event store; schedule-based GC in snapshot store; hygiene workflows in recipes.
- Integrate persistence metrics into observability layer and add alert thresholds.
- Document retention, backup, and incident response procedures.
- Run fault-injection drills (storage offline, quota exhaustion) and capture results.

**Exit criteria**: Persistence modules pass fault drills; governance docs aligned; L4 status promoted to "production-ready".

### Phase D · External Interfaces & Final QA (Week 4-5)
- Implement L7 adapter authentication, rate limiting, dispatcher integration, and plugin sandbox execution path.
- Run comprehensive end-to-end regression suite including API-triggered workflows.
- Conduct security/privacy review and update redaction defaults.
- Produce release candidate notes, migration steps, and sign-off checklist.

**Exit criteria**: HTTP adapter and plugin sandbox gated behind policy with green tests; release candidate tag cut; completion checklist signed by product, infra, and QA.

## 4. Cross-Cutting Tasks

- **Documentation alignment**: keep layer status tables consistent (README, progress docs, release notes).
- **Automation & tooling**: refresh CI pipeline with per-layer test stages and artifact uploads.
- **Release management**: maintain changelog, version bump plan, and rollback strategy.
- **QA engagement**: schedule weekly triage, track defects in issue tracker, and publish burn-down dashboards.
- **Risk tracking**: maintain risk register (CDP stability, Chrome version drift, privacy compliance) with mitigation owners.

## 5. Iterative Development Approach

To keep the effort system-focused (not role-oriented), treat each phase as an iterative loop:

1. **Plan** – refine phase backlog, acceptance criteria, and test scope.
2. **Implement** – land code/doc/test changes; keep branches short-lived.
3. **Validate** – run targeted suites (unit/integration/headless/headful) and capture metrics.
4. **Review & Adjust** – update this plan, note risks, feed learnings into the next loop.

Move to the next phase only after exit criteria are met; otherwise stay in the loop and remediate.

## 6. Tracking & Updates

- Update this plan and the associated progress docs each Friday EOD.
- Share weekly status in #soulbrowser-release with highlights, blockers, and decisions.
- Trigger escalation if any phase slips by >3 working days.

---
*This document replaces the ad-hoc per-layer timelines and should be treated as the single source of truth for product completion planning.*
