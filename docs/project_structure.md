# Project Structure Overview

This document provides a guided tour of the SoulBrowser repository and explains the purpose of the major directories and source files. Use it as a quick reference when navigating the codebase.

## Top-Level Layout

| Path | Purpose |
|------|---------|
| `Cargo.toml` | Root Rust manifest referencing soul-base crates and local modules. |
| `Cargo_soul_integration.toml` | Historical manifest retained for comparison with the former soul-integration prototype. |
| `build.rs` | Build script hook (currently minimal/no-op). |
| `config/` | Runtime configuration artifacts (see below). |
| `docs/` | Project documentation: e.g., `soul_base_components.md` (integration overview), `project_structure.md` (this file), `l0_development_plan.md` (runtime & adapters roadmap). |
| `examples/` | Placeholder for future examples (currently empty after removing legacy demos). |
| `src/` | Active CLI/library source code (detailed in the next section). |
| `tests/` | Integration and smoke tests (feature-gated `full-stack` suites). |
| `target/` | Cargo build artifacts (ignored by git). |

### `config/`

| File/Dir | Purpose |
|----------|---------|
| `config.yaml.example` | Sample configuration showing how to declare `policy_paths`, `strict_authorization`, and other defaults. Copy to `config.yaml` for local overrides. |
| `policies/browser_policy.json` | Default soul-base route policy used when `SOUL_POLICY_PATH` is not set. Defines `navigate`, `click`, `type`, and `screenshot` bindings. |

## `src/` Modules

| Path | Role |
|------|------|
| `main.rs` | CLI entry point. Parses commands, loads config, applies env overrides, and dispatches to the modules below. |
| `lib.rs` | Library facade re-exporting key types (`Browser`, `BrowserConfig`, `BrowserType`). |
| `app_context.rs` | Creates shared runtime state (storage, auth manager, tool registry) using soul-base components; reused across CLI commands. |
| `auth.rs` | Soul-base authentication, authorization, and quota wiring. Handles policy normalization, strict-mode fallback, and audit logging. |
| `browser_impl.rs` | Core orchestration layer (L0/L1). Builds `L0Protocol`, `L1BrowserManager`, and exposes `Browser`/`Page` wrappers backed by soul-base crates. |
| `config.rs` | Simple wrapper for `BrowserConfiguration` (use soulbase-config) and configuration helpers. |
| `errors.rs` | Unified error type based on `soulbase-errors`. |
| `interceptors.rs` | Defines the interceptor chain stages (soul-base standard stages + logging/resilience/rate-limit/policy enforcement). |
| `policy.rs` | Loads route policy specifications from disk/ENV, exposes helpers for merging attributes. |
| `storage.rs` | Adapter around `soulbase-storage` for session/event persistence.
| `tools.rs` | Boots the soul-base tool registry and provides helper APIs to execute tools. |
| `types.rs` | Shared type definitions (subjects, sessions, etc.) largely derived from `soulbase-types`. |
| `soul_direct.rs` | Miscellaneous examples of calling soul-base crates directly (kept for developer reference). |
| `analytics/mod.rs` | Session analytics/reporting scaffolding (leverages storage events; still evolving). |
| `automation/mod.rs` | Automation engine placeholder (current CLI `run` command uses this; will be expanded with soul-base flows). |
| `export/mod.rs` | Implements data exporters (JSON/CSV/HTML) that read from soul-base storage. |
| `replay/mod.rs` | Session replay helper backed by soul-base storage and browser orchestration. |

## `tests/`

- `soul_base_integration_test.rs` exercises the minimal soul-base wiring (policy + start command).
- `integration_test.rs`, `e2e_test.rs`, `stress_test.rs` are feature-gated (`--features full-stack`). They reference higher-level crates and will pass once those dependencies are reintroduced or mocked.

## Helpful References

- `docs/soul_base_components.md` — deeper explanation of how each soul-base crate is used and how to configure strict mode/policies.
- `docs/l0_development_plan.md` — progress tracker for runtime/adapters.
- `docs/l1_development_plan.md` — roadmap for the upcoming unified kernel work.
- `MIGRATION_GUIDE.md` — historical record of the migration steps and future todo list.
- `INTEGRATION_SUMMARY.md` — snapshot of what’s fully integrated vs. pending in the CLI.

This structure should help you quickly locate the module you need—whether you’re adjusting policies, extending automation commands, or wiring in additional soul-base crates.
