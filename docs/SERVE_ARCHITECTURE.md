# Serve Architecture

This note documents how `cargo run --bin soulbrowser -- serve ...` wires the
console API so future cleanup can follow the same path.

## 1. CLI Entry → `cmd_serve`

`src/main.rs` parses the `serve` subcommand and hands it to
`src/cli/serve.rs::cmd_serve`. Important steps:

1. Parse CLI/env/config overrides (LLM cache, shared perception pool,
   `--surface` preset, auth tokens, etc.).
2. Normalize the requested tenant id and create the tenant storage root via
   `tenant_storage_path`.
3. Initialize shared resources: `AppContext`, `ServeState`, rate limiter, health
   probes, and chat context semaphore.
4. Run startup readiness checks and log whether Serve will enforce strict auth
   (`--disable-auth` short-circuits this step).

## 2. Tenant Context & Storage Layout

`ServeState` owns both the `AppContext` handle and the storage roots:

- `default_storage_root` → `soulbrowser-output/tenants/<tenant>/tasks/` for
  persisted plans.
- `execution_output_root` → `soulbrowser-output/tenants/<tenant>/executions/`
  for chat/task artifacts (`plans.json`, `executions.json`, telemetry, etc.).

CLI utilities now accept `--tenant` so the same layout is used for
`run`/`replay`/`export`/`analyze`/`chat`/`memory`/`observations`/`scheduler`/
`policy`/`timeline`/`gateway`/`start`/`record`. Use `scripts/migrate_execution_outputs.sh`
if legacy bundles still live under `soulbrowser-output/tasks/`.

## 3. Router Composition & Auth Layers

`src/server/router.rs` exposes `ServeRouterModules`, allowing Serve to load only
selected modules:

- `ServeSurfacePreset::Console` (default) merges perception/chat/tasks/memory/
  plugins/self_heal/admin.
- `ServeSurfacePreset::Gateway` exposes only perception/chat/tasks so a public
  Serve doesn’t leak operator-only routes.

`cmd_serve` resolves the preset from CLI → config (`serve_surface`) → env
(`SOUL_SERVE_SURFACE`).

The router tree looks like:

- `/` + `/health` + `/livez` + `/readyz` + `/metrics` served directly from
  `static/console.html` (with `/assets/*` provided by `static/assets/`).
- `/api/*` merges the modules chosen by the preset; optional middlewares add IP
  filtering and token auth.
- `/ws/*` exposes the websocket shell for console streaming.

## 4. Background Maintenance & Rate Limiting

`cmd_serve` wires several recurring jobs before axum starts listening:

- Plan/output TTL pruning based on `SOUL_PLAN_TTL_DAYS`/
  `SOUL_OUTPUT_TTL_DAYS`.
- Rate limiter bucket GC honoring `SOUL_RATE_LIMIT_BUCKET_TTL_SECS` and
  `SOUL_RATE_LIMIT_GC_SECS`.
- Optional shared perception pooling and LLM cache initialization.

The server also exposes:

- `/metrics` → Prometheus scrape endpoint.
- `/api/tasks/:id/stream` → SSE relay driven by `TaskStatusRegistry`.
- `/api/tasks/:id/executions` → reads tenant scoped execution bundles.

Keep this flow in mind whenever we add new modules or storage locations so (a)
Serve still starts through `cmd_serve`, (b) tenant paths stay under
`soulbrowser-output/tenants/<tenant>/`, and (c) router presets/Gateway mode are
updated with explicit intent.
