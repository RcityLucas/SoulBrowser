# Performance & Security Checklist

Snapshot of the remaining items required by Phase 5 (Polish & Documentation). Each section lists the current state and concrete next steps so we can schedule profiling passes and security reviews without digging through the entire codebase.

## Performance

| Area | Current Signals | Next Actions |
|------|-----------------|--------------|
| **Perception Service** | `/api/perceive` already logs `avg_duration_ms` and shared-session hit/miss/failure metrics (see `src/perception_service.rs` + `serve_perceive_handler`). | ✅ `scripts/perception_bench.sh` now runs 20 shared + 20 ephemeral jobs via the CLI and writes `soulbrowser-output/perf/perception.csv`. Remaining: plot helper + README blurb on interpreting results. |
| **Task Execution** | `TaskStatusRegistry` tracks per-step timestamps, but we never emit aggregated latency stats. | ✅ `FlowExecutionReport` now carries `metrics` (totals/avg/max wait+run). New CLI: `cargo run -- metrics execution --report path/to/report.json` prints a summary from saved run bundles. Next: wire this into `/api/tasks/:id` detail payload once execution data is persisted. |
| **LLM Planner** | No caching; every `/api/chat` / `/api/tasks` hit calls the provider. | ✅ `ChatRunner` now consults an optional disk cache (`SOULBROWSER_LLM_CACHE_DIR` or `--llm-cache-dir`) before calling the provider. Cached hits reuse stored `AgentPlan`s for both plan + replan flows. Remaining: surface hit/miss counters under `/metrics`. |
| **Web Console** | React tables re-render entire dataset on every poll; acceptable for ≤100 tasks. | Switch `/api/tasks` poll interval to 5 s (currently 2 s) and memoize columns/rows; add virtualization if task count > 500. |

## Security

| Area | Current Signals | Next Actions |
|------|-----------------|--------------|
| **Rate Limiting** | Axum router allows unlimited `/api/chat` or `/api/tasks` posts. | ✅ Added per-IP token buckets (default 30 chat + 15 task creations/min). Configurable via `SOULBROWSER_RATE_LIMIT_CHAT` / `SOULBROWSER_RATE_LIMIT_TASKS`. |
| **API Input Validation** | `CreateTaskRequest` accepts arbitrary strings, no length or URL validation. | Enforce: `prompt <= 2000 chars`, `constraints <= 10 entries`, `current_url` must start with `http://` or `https://`. Return `400` with friendly error. |
| **Secrets Handling** | Task plans stored on disk may contain inline API keys if user pastes them. | Before calling `TaskPlanStore::save_plan`, scrub known key patterns (e.g., `sk-`, `ANTHROPIC_API_KEY`) from `prompt`/`constraints`/metadata. Track scrub count in logs. |
| **Transport Security** | Dev server only binds HTTP. | Document `SOULBROWSER_TLS_CERT` / `SOULBROWSER_TLS_KEY` plan (or use reverse proxy) so production deploys terminate TLS before hitting Axum. |

Once these items are closed, we can mark the Phase 5 performance/security bullet as complete in `INTEGRATION_ROADMAP.md`.
