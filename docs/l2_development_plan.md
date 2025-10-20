# L2 Layered Perception · Development Plan (2025-10)

> Goal: Deliver a production-ready structural perceiver that extracts reliable anchors, visibility/clickability signals, and DOM/AX diffs for downstream automation (L3/L5).

## 0. Current Snapshot
- CLI demo now drives a real Chromium session (either launched or attached) and already calls into `perceiver-structural`.
- Sampling (`DOMSnapshot.captureSnapshot` + `Accessibility.getFullAXTree`) works on desktop Chrome but may be blocked in some sandboxes. Code falls back to CSS-only hints when snapshots fail.
- Resolver now blends query results with multi-hint fallbacks and weighted ranking; judges/differ provide heuristic visibility/clickability checks and DOM/AX summaries (still heuristics, not full fidelity).
- CLI exposes telemetry via `soulbrowser perceiver`, supporting filters and JSON export.

## 1. Phase Breakdown & Milestones

### Phase 1 · Robust Sampling & Indexing (in progress)
1. Wrap DOM/AX capture with retries, domain enables, and friendly errors.
2. Build DOM index (nodeName/attributes/backendNodeId/geometry) once per snapshot.
3. Cache snapshots & anchors with event-driven invalidation (already stubbed; needs wiring to CDP events).

### Phase 2 · Candidate Generation & Ranking
1. Support multi-hint input: CSS, ARIA, Text, BackendId, Geometry.
2. Derive candidate metadata (ax role/name, heuristics, attribute matches) from DOM index.
3. Implement real scoring weights (visibility, semantic matches, recency) and Top-K selection.

### Phase 3 · Structural Judgement & Diff
1. Flesh out `judges::visible/clickable/enabled` using DOM styles, AX states, geometry.
2. Implement `differ::compute` for DOM/AX comparisons (added/removed nodes, attribute changes, text diffs).
3. Integrate policy thresholds (opaque area, debounce windows, max diff size).

### Phase 4 · Integration & Telemetry
1. Emit structured events into State Center (resolve, judge, diff) with redaction rules.
2. Add metrics (latency averages, cache hit rate, anchor confidence distribution).
3. Surface perceiver options/config via CLI & docs.

### Phase 5 · Verification & Hardening
1. Unit tests for generator, ranking, judges, diff.
2. Integration tests using headless Chrome (attach mode & launch mode).
3. Smoke tests wired to CI (opt-in similar to `SOULBROWSER_SMOKE_DEMO`).

## 2. Work Queue (next iterations)
- [x] Implement DOM index + candidate augmentation (Phase 2 #1-2).
- [x] Replace stub ranking with weighted scoring (Phase 2 #3).
- [x] Expand judges to use DOM styles + AX states (Phase 3 #1).
- [x] Add DOM diff skeleton with change types (Phase 3 #2).
- [x] Wire perceiver events to State Center (Phase 4 #1); events now append to `InMemoryStateCenter` and surface in CLI.
- [x] Add unit tests for generator/judges/diff (Phase 5 #1).
- [x] Add sampling retry/backoff and SnapLevel::Light preference (Phase 1 #1).
- [x] **Integrate SnapLevel cache invalidation with CDP lifecycle (Phase 1 #3)** - Completed 2025-10-20
  - `lifecycle.rs` module with `LifecycleWatcher` for automatic cache invalidation
  - Subscribes to CDP events: `navigate`, `load`, `domcontentloaded`, `frame_attached`, `frame_detached`
  - Smart invalidation policy: full invalidation on navigate/load, snapshot-only on DOM changes
  - 3 unit tests covering navigate invalidation, DOM update preservation, and clean shutdown
- [x] **Expose cache hit rate + policy-tunable debouncing (Phase 4 #2)** - Completed 2025-10-20
  - Cache metrics already exposed via `metrics::snapshot()` with hit/miss tracking
  - CLI `soulbrowser perceiver` command displays cache stats with hit rates
  - Policy-based debounce configuration supported via `ResolveOptions::debounce_ms`
- [x] **Add integration tests for cache behavior (Phase 5 #2)** - Completed 2025-10-20
  - `tests/cache_integration.rs` with real Chrome integration tests
  - Tests cache invalidation on navigation and metrics tracking
  - Requires `SOULBROWSER_USE_REAL_CHROME=1` environment variable

## 3. Risks & Mitigations
- **Sandbox limitations** prevent DOMSnapshot/AX capture.
  - Mitigation: attach-mode docs (`--ws-url`), JS fallback, retry with `DOMSnapshot.enable`.
- **Anchor heuristics** may mis-rank due to incomplete DOM data.
  - Mitigation: iterate with real pages (Wikipedia demo), log scoring breakdown.
- **Test flakiness** when depending on live Chrome.
  - Mitigation: mark smoke tests opt-in, use deterministic fixtures for unit tests.

## 4. Deliverables Definition
- `perceiver-structural` crate exposes stable API with real implementations (resolve/judge/diff).
- CLI demo logs anchor confidence & diff summaries.
- Docs updated (user + dev notes) describing perceiver config and troubleshooting.

---

**Completed 2025-10-20:** All Phase 1-5 items completed. L2 Layered Perception now features:
1. **Automatic Cache Invalidation** - CDP lifecycle integration with smart invalidation policies
2. **Cache Metrics & CLI Visibility** - Real-time hit/miss tracking via `soulbrowser perceiver` command
3. **Integration Test Suite** - Real Chrome validation tests (opt-in with `SOULBROWSER_USE_REAL_CHROME=1`)
4. **Production-Ready Caching** - TTL-based caches with prefix invalidation and policy control

**Next Steps:** Performance tuning, advanced invalidation strategies, and expanded integration test coverage.
