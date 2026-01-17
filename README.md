# SoulBrowser

> Intelligent web automation with multi-modal perception, policy-guarded execution, and plug-in ready gateways.

SoulBrowser stitches together a command-line facade, a reusable kernel, and a large set of focused crates to plan, execute, and inspect browser automation flows. The workspace already ships with structural/visual/semantic perceivers, a policy-aware scheduler, memory and timeline services, and an HTTP surface that can expose the console UI or act as a gateway for external adapters.

Need more context? Use these companions:
- `README_CN.md` &mdash; Chinese overview for the same repo.
- `docs/README.md` &mdash; module cheat sheet.
- `docs/module_deep_dive.md` &mdash; crate-by-crate architecture notes.

## Table of contents
1. [Highlights](#highlights)
2. [Architecture overview](#architecture-overview)
3. [Repository layout](#repository-layout)
4. [Module map](#module-map)
5. [CLI surfaces](#cli-surfaces)
6. [Quick start](#quick-start)
7. [Configuration and environment](#configuration-and-environment)
8. [Data and storage conventions](#data-and-storage-conventions)
9. [Observability & diagnostics](#observability--diagnostics)
10. [Development workflow](#development-workflow)
11. [Troubleshooting tips](#troubleshooting-tips)
12. [Status and roadmap notes](#status-and-roadmap-notes)
13. [Licensing](#licensing)

## Highlights
- **Unified CLI** &mdash; `soulbrowser` wraps every subsystem: multi-modal perception, L8 agent chat, scheduler/policy inspection, artifact export, memory/timeline introspection, and serve/gateway surfaces.
- **Kernel + AppContext** &mdash; `soulbrowser-kernel` wires Chrome/CDP access, perception services, the registry, scheduler, policy center, plugin registry, memory center, and storage providers (defaulting to the soulbase integration).
- **Perception stack** &mdash; Structural, visual, and semantic perceivers (plus the `perceiver-hub`) form the basis for the `perceive` command and console overlays, producing `MultiModalPerception` documents and screenshots.
- **Action pipeline** &mdash; `action-primitives` feed the locator/gate/flow crates, which the scheduler uses to dispatch tool calls via the registry and policy center.
- **Observability** &mdash; The state center caches perceiver and dispatch events, the event store exposes hot/cold rings and timeline exports, and metrics are served over HTTP (default `localhost:9090`).
- **Security & governance** &mdash; Policy and permission brokers, privacy filters, plugin guardrails, and network-tap scaffolding are ready for L7 adapters, plugins, and governance timelines.

## Architecture overview
```
CLI (soulbrowser)
   │
   ├─ Runtime bootstrap (env/config, metrics, logging)
   │
   └─ Dispatch ─▶ Kernel / ServeOptions ─▶ AppContext cache
                    │
                    ├─ Registry + Scheduler + State Center
                    ├─ PerceptionService → Perceiver Hub (structural/visual/semantic)
                    ├─ Event Store / Timeline / Memory Center
                    └─ Gateway (L7 adapters, HTTP surface, plugins, policies)
```
Key supporting crates:
- **Kernel runtime** (`soulbrowser-kernel`): Serve/gateway orchestration, app context builder, perception service, storage/tasks, metrics, server surfaces.
- **Action layer** (`action-primitives`, `action-locator`, `action-gate`, `action-flow`, `soulbrowser-actions`): execution primitives, selector healing, gating evidence, flow orchestration.
- **Perception layer** (`perceiver-structural`, `perceiver-visual`, `perceiver-semantic`, `perceiver-hub`, `network-tap-light`): multi-modal DOM/AX, screenshot/visual diff, semantic analysis, network summaries.
- **Control plane** (`registry`, `scheduler`, `state-center`, `policy-center`, `event-bus`, `event-store`, `snapshot-store`, `memory-center`): sessions/pages, dispatch queues, policy snapshots/overrides, and historical data.
- **Integration layer** (`cdp-adapter`, `permissions-broker`, `extensions-bridge`, `stealth`, `integration-soulbase`, `l6-*`, `l7-*`): CDP + permission scaffolds, plugin runtime, observability adapters, HTTP surfaces.
- **Agent & LLM support** (`agent-core`, `soulbrowser-kernel::agent`, `chat_support`, `l6-observe`, `l6-privacy`): plan models, validators, planner selection, LLM caching, privacy filtering, and observation exporters.

The CLI and Serve/Gateway surfaces sit on top of this stack: each command parses env/config overrides, requests an `AppContext`, and interacts with the scheduler, registry, and perception layers through kernel APIs. Because the same runtime powers CLI sessions, HTTP adapters, and plugins, diagnostics collected by `state-center`, `event-store`, or `metrics` are instantly visible everywhere.

## Repository layout
| Path | Purpose |
| --- | --- |
| `src/cli/` | CLI commands (`serve`, `perceive`, `chat`, `analyze`, `artifacts`, `console`, `scheduler`, `policy`, `timeline`, etc.). |
| `crates/soulbrowser-kernel/` | Core kernel: serve/gateway, runtime, perception service, config, app context, metrics, auth, storage, tools. |
| `crates/action-*` | Action primitives, locator, gate, flow & executor stacks used by the scheduler. |
| `crates/perceiver-*` & `crates/perceiver-hub/` | Structural/visual/semantic perceivers plus the hub that fuses them into `MultiModalPerception`. |
| `crates/agent-core/` | L8 agent plan models, LLM provider abstraction, rule/LLM planner plumbing, plan-to-flow conversion. |
| `crates/registry/`, `crates/scheduler/`, `crates/state-center/`, `crates/policy-center/` | Orchestration backbone: tracks sessions, dispatch queues, policy snapshots/overrides, and execution/perception events. |
| `crates/event-store/`, `crates/l6-timeline/`, `crates/memory-center/`, `crates/event-bus/` | Event persistence, cold/hot rings, timeline export, lightweight memory persistence, and pub/sub utilities. |
| `crates/cdp-adapter/`, `crates/network-tap-light/`, `crates/permissions-broker/`, `crates/extensions-bridge/`, `crates/stealth/` | L0 interaction scaffolds (Chrome detection, network tapping, permission enforcement, extension channels, stealth helpers). |
| `crates/l7-adapter/`, `crates/l7-plugin/`, `crates/integration-soulbase/`, `crates/soulbrowser-actions/` | Gateway/router wiring, plugin runtime, integration provider, and action re-exports. |
| `config/` | Sample configuration, policy, planner, plugin, and permission bundles (`config/README.md` documents usage). |
| `docs/` | Chinese overview plus module overview (`docs/README.md`) and deep dive (`docs/module_deep_dive.md`). |
| `static/` | Static assets (console shell HTML). |
| `soulbrowser-output/` | Default runtime output root (tenant storage, run bundles, state center snapshots, artifacts). |

### Root level quick reference
- `Cargo.toml` / `Cargo.lock` – workspace manifest + lockfile.
- `build.rs` – embeds build metadata (timestamp, git hash/branch).
- `src/` – binary/lib sources.
- `crates/` – internal libraries powering the CLI.
- `config/` – sample configs, policies, and env overrides.
- `docs/` – onboarding/architecture documentation.
- `static/console.html` – console shell served by `serve --surface console`.
- `third_party/` – vendored assets or placeholder for external deps.
- `soulbrowser-output/` – default runtime output tree (git-ignored).
- `target/` – Cargo build artifacts (generated).

### `crates/soulbrowser-kernel/src` key modules
- `kernel.rs` – serve/gateway wiring, HTTP pipeline, tenant normalization.
- `runtime.rs` – runtime bootstrap, tenant storage prep, rate limits.
- `app_context.rs` – shared storage/auth/tool managers, scheduler/service wiring.
- `perception_service.rs` – orchestration of structural/visual/semantic perceivers.
- `metrics.rs` – Prometheus registry/recorders.
- `gateway/`, `server/`, `integration/`, `automation/`, `policy/`, etc. – supporting modules for adapters, tools, auth, storage, analytics, chat, watchdogs.

## Module map
Summaries below are mirrored (with more detail) in `docs/README.md` and `docs/module_deep_dive.md`.

| Layer | Crates / Paths | Highlights |
| --- | --- | --- |
| CLI shell | `src/cli/` | `app.rs` bootstrap, `runtime.rs` config/env, `commands.rs` + `dispatch.rs` for verbs, per-command files (`serve`, `chat`, `perceive`, etc.). |
| Kernel/runtime | `crates/soulbrowser-kernel` | `Kernel::serve/gateway`, `AppContext`, `ServeState`, `PerceptionService`, metrics, HTTP surfaces. |
| Action & scheduler | `action-*`, `soulbrowser-actions`, `registry`, `scheduler`, `state-center` | Navigation/click primitives, locator healers, validation gates, dispatcher/orchestrator, ExecRoute, dispatch/probe events. |
| Perception | `perceiver-*`, `perceiver-hub`, `network-tap-light` | DOM/AX resolution, screenshot/OCR, semantic analysis, hub aggregation, network summaries. |
| Control plane | `event-store`, `l6-timeline`, `policy-center`, `permissions-broker`, `memory-center`, `integration-soulbase` | Event persistence, timeline export, runtime policy overrides, permission enforcement, memory persistence, storage/auth/tool providers. |
| Governance & adapters | `cdp-adapter`, `extensions-bridge`, `l7-adapter`, `l7-plugin`, `l6-privacy`, `l6-observe`, `stealth` | Chrome detection, extension scaffolds, HTTP adapters, plugin runtime, privacy filters, observation exporters. |
| Agent/LLM | `agent-core`, `soulbrowser-kernel::agent`, `chat_support` | Agent plan models, planner selection (rule/LLM), plan-to-flow converters, execution reports. |

## CLI surfaces
| Command | Status | Description |
| --- | --- | --- |
| `serve` | Stable | Runs the testing server and console UI; supports authentication, tenant isolation, shared session pool toggles, and console/gateway presets. |
| `gateway` | Stable | Starts the L7 adapter HTTP surface (future gRPC/WebDriver) with optional policy files and demo plan execution. |
| `perceive` | Stable | Performs multi-modal perception against a URL, emitting summaries, logs, JSON, and optional screenshots. |
| `chat` | Stable | Generates L8 agent plans (rule or LLM planners), optionally executes flows via the scheduler, and persists run bundles/artifacts. |
| `analyze` | Stable | Runs analytics (performance, accessibility, security, usability, compatibility, or full) against recorded sessions. |
| `artifacts` / `console` | Stable | Inspect run-bundle artifacts, extract payloads, render BrowserUse-style GIF timelines, and launch an ad-hoc console viewer bound to bundle data. |
| `timeline` | Stable | Hydrates the event store from storage, exports governance timelines/replays/records, and writes logs to disk. |
| `scheduler`, `perceiver`, `policy`, `info` | Stable | Debug/observability commands to inspect dispatch queues, perceiver events, policy revisions/overrides, and overall health. |
| `tools` | Beta | Register/list/remove runtime tool descriptors (JSON) and persist them under `config/tools`. |
| `telemetry` | Beta | `telemetry tail`, `telemetry webhook/posthog/list/remove` manage sinks defined in `config/telemetry.json`; set `SOULBROWSER_TELEMETRY_STDOUT=1` to emit events. |
| `config` | Stable | View, set, reset, or validate the YAML config file; uses JSON dot-path assignments. |
| `run`, `record`, `replay`, `export`, `start`, `demo` | Retired | Historical commands now bail with guidance to use the consolidated `serve`/`gateway` surfaces. |
 
### Common flows
```bash
# Build everything once
cargo build --workspace

# Launch the console surface (http://127.0.0.1:8787)
cargo run --bin soulbrowser -- serve --port 8787 --surface console --auth-token devtoken

# Run perception with screenshot + JSON summary
cargo run --bin soulbrowser -- perceive --url https://example.com --all \
    --screenshot ./soulbrowser-output/perception/example.png \
    --output ./soulbrowser-output/perception/example.json

# Generate an agent plan and execute it immediately
cargo run --bin soulbrowser -- chat --prompt "Book a round-trip ticket" --execute \
    --save-run ./soulbrowser-output/runs/ticket.json --artifacts-path ./soulbrowser-output/runs/artifacts.json

# Inspect recent scheduler/perceiver state
cargo run --bin soulbrowser -- scheduler --status failure
cargo run --bin soulbrowser -- perceiver --kind resolve --format table
```

## Quick start
1. **Install prerequisites** – Rust 1.70+, Chrome/Chromium (or set `SOULBROWSER_CHROME`), and any LLM credentials you plan to use.
2. **Clone & build** – `git clone … && cd soulbrowser && cargo build --workspace` to download dependencies and compile every crate.
3. **Copy config** – `cp config/config.yaml.example config/config.yaml`, then adjust browser defaults, output directory, and policy paths.
4. **Set secrets/env** – duplicate `config/local.env.example` to `config/local.env`; populate `SOUL_CONSOLE_TOKEN`, `OPENAI_API_KEY`, etc. (CLI auto-loads the file before parsing args)。如需备用 OpenAI key，可一起填写 `SOULBROWSER_OPENAI_BACKUP_KEY`，planner 会在主 key 被限流时自动切换。
5. **Start Serve** – `cargo run --bin soulbrowser -- serve --surface console --port 8787 --auth-token devtoken` and open the printed URL.
6. **Iterate** – use `perceive`, `chat`, `scheduler`, and `perceiver` to capture perception data, agent flows, and debugging information under `soulbrowser-output/`.

## Configuration and environment
1. Copy `config/config.yaml.example` to `config/config.yaml` and adjust:
   - `default_browser`, `default_headless` – CLI defaults for Chromium/Chrome.
   - `output_dir` – root for artifacts/run bundles (`./soulbrowser-output` by default).
   - `soul` block – enable agent assistance, model id, API key, prompt directory.
   - `recording` / `performance` – toggle screenshot/video/network/perf capture.
   - `policy_paths` – list of policy files loaded into the policy center (`SOUL_POLICY_PATH` mirrors the first entry).
   - `strict_authorization` / `serve_surface` – default auth + surface for `serve`.
2. Populate `config/local.env` (ignored by git) to set secrets (OpenAI keys, console tokens, etc.). `src/cli/runtime.rs` automatically loads it before parsing CLI args.
3. Optional directories under `config/`: `permissions/` (per-tenant gateway policies), `plugins/` (registry + manifests), `planner/` (prompt templates), `policies/` (org-specific policy bundles).
   - Telemetry sinks persist under `config/telemetry.json` (managed via `soulbrowser telemetry ...` commands); the CLI auto-loads this file to register webhook/PostHog sinks on startup.

### Key environment variables
| Variable | Purpose |
| --- | --- |
| `SOULBROWSER_CHROME` / `SOULBROWSER_USE_REAL_CHROME` | Override or force the Chrome/Chromium binary the CDP adapter launches. |
| `SOULBROWSER_CHROME_PROFILE` | Provide a Chrome profile directory (else `.soulbrowser-profile*` temp dirs are used). |
| `SOULBROWSER_WS_URL` | Attach to an existing DevTools websocket instead of launching Chrome (mirrors CLI `--ws-url`). |
| `SOULBROWSER_DISABLE_PERCEPTION_POOL` | Disable shared perception sessions (also settable via `serve --shared-session=false`). |
| `SOULBROWSER_LLM_CACHE_DIR` | Custom cache folder for planner outputs (CLI `--llm-cache-dir` overrides). |
| `SOUL_STRICT_AUTHZ` | Forces strict authorization (auto-enabled when config demands or when serve auth tokens are supplied). |
| `SOUL_SERVE_SURFACE` | Default serve surface (`console` or `gateway`). |
| `SOUL_CONSOLE_TOKEN` / `SOUL_SERVE_TOKEN` | Auth tokens accepted by the serve surface without passing `--auth-token`. |
| `SOUL_POLICY_PATH` | Explicit policy snapshot path if not relying on config search paths. |
| `SOUL_CHAT_CONTEXT_LIMIT` / `SOUL_CHAT_CONTEXT_WAIT_MS` | Tune concurrency and wait time for chat context captures. |

## Data and storage conventions
- `soulbrowser-output/` (configurable) hosts tenant-specific directories, artifacts, screenshots, logs, run bundles, state-center snapshots, and timeline exports.
- Run bundles captured via `chat --save-run` include `plans`, `execution`, `state_events`, and `artifacts`, which the `console` and `artifacts` commands ingest.
- `memory_center` records live in-memory unless pointed at a persistence file. Use `CliContext::app_context()` → `memory_center()` to store/retrieve small facts.
- Event streams are stored via `event-store` hot rings and optional cold writer (see `config/defaults` when tuning retention).
- Perceiver logs/overlays, scheduler snapshots, and CLI exports land under `soulbrowser-output/<tenant>/` to keep per-tenant state isolated.
- Runtime tool descriptors live in `config/tools/*.json` (managed via `soulbrowser tools register/remove`); each CLI run auto-loads these definitions into the planner/tool registry.

## Observability & diagnostics
- **Metrics** – the CLI spawns a Prometheus endpoint on `http://localhost:<metrics_port>/metrics` (default `9090`). Scheduler, registry, CDP adapter, LLM cache, and plan statistics are exposed.
- **State center** – stores recent dispatch/perceiver events for CLI inspection and console overlays. Snapshots persist under `soulbrowser-output/state-center/` when enabled.
- **Timeline exports** – `soulbrowser timeline` hydrates an in-memory event store and writes records/timelines/replays based on action/flow/task identifiers or time ranges.
- **Artifacts & console** – run bundles can be inspected offline (`soulbrowser console --serve --input bundle.json`) and artifacts filtered/extracted or turned into GIF timelines (`soulbrowser artifacts --gif timeline.gif ...`).
- **Logs** – tracing logs stream to stdout and respect `--log-level`/`--debug`. `config/local.env` can set `RUST_LOG` (e.g., `soulbrowser_kernel=debug`) for fine-grained filters.
- **Telemetry stream** – set `SOULBROWSER_TELEMETRY_STDOUT=1` to emit JSON step/task events during execution; sinks are pluggable (`soulbrowser_kernel::telemetry::register_sink`) for webhooks/PostHog adapters.
- `soulbrowser telemetry tail` 可以实时查看事件，使用 `soulbrowser telemetry webhook --url …` 发送到任意 HTTP endpoint，或用 `soulbrowser telemetry posthog --api-key ...` 注册 PostHog sink；每个事件包含基础运行指标（`runtime_ms`、LLM token 统计若模型返回 usage 即自动填充）。

## Development workflow
1. **Format & lint**
   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   ```
2. **Test**
   ```bash
   cargo test --workspace
   ```
3. **Targeted runs**
   - `cargo run --bin soulbrowser -- serve ...` for end-to-end smoke tests.
   - `cargo run -p perceiver-hub --example ...` when iterating on perceiver logic.
   - `cargo test -p <crate>` for crate-specific changes (action stack, scheduler, policy center, etc.).
4. **Artifacts**
   - Use `soulbrowser chat ... --save-run` to capture reproducible bundles.
   - `soulbrowser console --serve --input bundle.json` to visualize plan/execution timelines without a live browser.

### Testing & QA tips
- Keep `cargo fmt`/`cargo clippy -D warnings` clean to mirror CI.
- Use `assert_cmd` (already a dev-dependency) for CLI regression tests.
- For scheduler/perceiver debugging, record `soulbrowser-output/state-center/snapshot.json` (if persistence is enabled) and share run bundles.
- Run `gateway` with `--disable-auth` only for local experiments; production instances should rely on tokens/IP allowlists.

## Troubleshooting tips
| Symptom | Suggested checks |
| --- | --- |
| Serve fails to bind port | Ensure no other process uses the port; pass `--port <freeport>` or set `--metrics-port 0` if Prometheus conflicts. |
| Chrome cannot be found | Set `SOULBROWSER_CHROME=/path/to/chrome` or install Chromium/Chrome; `cdp-adapter` logs detection attempts. |
| Perception hangs | Lower `--timeout`, disable pooling via `--shared-session=false` (Serve) or set `SOULBROWSER_DISABLE_PERCEPTION_POOL=1`, and confirm DevTools websocket (`--ws-url`). |
| Planner errors due to LLM auth | Supply `--llm-api-key`, set `OPENAI_API_KEY`/`ANTHROPIC_API_KEY` in `local.env`, or fall back to `--llm-provider mock`. |
| Empty scheduler output | Ensure commands generate events (e.g., run `chat --execute` first) and check `soulbrowser-output/state-center/` snapshots. |

## Status and roadmap notes
- The CDP adapter, network tap, permissions broker, and extensions bridge are scaffolds; real Chromium wiring, audit hooks, and extension channels land in later milestones.
- Legacy commands (`start`, `run`, `record`, `replay`, `export`, `demo`) intentionally bail with guidance to the serve/gateway surfaces after the CLI refactor.
- The gateway currently exposes only the HTTP surface; gRPC and WebDriver listeners are stubbed behind CLI flags for future releases.
- LLM planner selection supports rule-based, OpenAI, Anthropic, or mock providers. Provide API keys via env vars or `chat` CLI overrides.
- Plugin registries, policy overrides, and privacy filters live under `crates/l7-plugin`, `crates/policy-center`, and `crates/l6-privacy`; populate `config/plugins` and `config/policies` as your org rolls out governance requirements.
- Documentation roadmap – `README_CN.md` and the `docs/` tree mirror this README; update them whenever new modules or workflows land to keep onboarding friction low.

## Licensing
Dual-licensed under MIT or Apache-2.0 (see `Cargo.toml` metadata). Choose the license that best fits your deployment.
