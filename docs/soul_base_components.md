# Soul-Base Component Integration

This document explains how SoulBrowser reuses the soul-base crates after the recent refactor, which modules call into them, and how to configure the integration in different environments.

## High-Level Overview

The CLI, examples, and automation APIs now delegate all critical functionality to soul-base crates:

| Capability | SoulBrowser module | Soul-base crate(s) |
|-----------|-------------------|--------------------|
| Authentication & Authorization | `src/auth.rs` | `soulbase-auth`, `soulbase-errors`, `soulbase-types` |
| Interceptor pipeline | `src/interceptors.rs` | `soulbase-interceptors`, `soulbase-errors` |
| Policy loading & normalization | `src/policy.rs` | soul-base policy DSL (embedded in `soulbase-interceptors`) |
| Storage / event persistence | `src/storage.rs` | `soulbase-storage`, `soulbase-types` |
| Tool execution | `src/tools.rs` | `soulbase-tools` |
| Shared types / IDs / subjects | `src/types.rs` | `soulbase-types` |
| Browser orchestration | `src/browser_impl.rs` | all of the above (L0/L1 wiring) |

The legacy `src/soul_integration/*` and migration shims were removed. Each SoulBrowser module owns direct calls into the relevant soul-base crate.

## Configuration & Environment

Configuration values can be provided via `config/config.yaml` (see `config/config.yaml.example`) or environment variables. Key settings include:

| Setting | Description | Default | Environment variable override |
|---------|-------------|---------|-------------------------------|
| `policy_paths` | Ordered list of policy JSON files | `config/policies/browser_policy.json` | `SOUL_POLICY_PATH` (first wins) |
| `strict_authorization` | Require facade approval without route-policy fallback | `false` | `SOUL_STRICT_AUTHZ=true` |
| `SOUL_QUOTA_PERSIST_MS` | Minimum interval between quota file writes (ms) | `2000` | `SOUL_QUOTA_PERSIST_MS` |
| `SOUL_QUOTA_REFRESH_MS` | Minimum interval between quota reloads (ms) | `30000` | `SOUL_QUOTA_REFRESH_MS` |

When the CLI starts it:

1. Loads the YAML configuration (or defaults).
2. Applies runtime overrides: `policy_paths` populate `SOUL_POLICY_PATH` if not already set, and `strict_authorization` toggles `SOUL_STRICT_AUTHZ`.
3. Passes the resolved policy list into `get_or_create_context`, which constructs `BrowserAuthManager::with_policy_paths`.

### Policy Files

`soulbase-auth` expects route policies to be described using the soul-base DSL. A starter file lives at `config/policies/browser_policy.json` and covers the core browser actions (`navigate`, `click`, `type`, `screenshot`). To add custom routes:

1. Copy the example file and extend it with additional `RoutePolicySpec` entries.
2. Point `policy_paths` (or `SOUL_POLICY_PATH`) to your custom file.
3. Optionally enable strict mode to require explicit approval from the facade.

### Strict Authorization Mode

By default `BrowserAuthManager` will fall back to allowing a request if the route matches policy specs but the facade returns a deny (useful during incremental rollout). Setting either the config flag or `SOUL_STRICT_AUTHZ=true` disables this fallback. Audit logs (`target="auth.audit"`) clearly annotate whether strict mode permitted or rejected a request.

### Quota Persistence

`FileQuotaStore` now throttles disk writes and refreshes:

- `SOUL_QUOTA_PERSIST_MS` controls how frequently the quota snapshot is persisted.
- `SOUL_QUOTA_REFRESH_MS` governs how often the in-memory map is reloaded from disk.
- Audit events (`target="auth.quota"`) log `allowed` vs `rate_limited` outcomes along with usage counts.

## Runtime Modules

- **`src/app_context.rs`**: Creates shared instances (storage, auth manager, tool registry) by calling soul-base constructors. All CLI entry points reuse the same context.
- **`src/browser_impl.rs`**: Wires together the soul-base L0/L1 layers (`L0Protocol`, `L1BrowserManager`, `Browser`, `Page`).
- **`src/interceptors.rs`**: Adds soul-base standard stages (context init, tenant guard, schema guard, error normalization, route policy) plus logging, resilience, rate limiting, and policy enforcement.
- **`src/tools.rs`**: Boots the soul-base tool registry and exposes an async interface to execute tools.
- **`src/storage.rs`**: Wraps the `soulbase-storage` backends for session/event persistence.
- **`src/auth.rs`**: Handles token normalization, delegated authorization, and quota checks against soul-base components.

## Building & Testing

- `cargo test auth::tests::test_browser_auth` still covers the policy/quota paths. The historical `full-stack` test suite now lives under `docs/examples/legacy_code/tests/` if you need the old coverage.

## Future Enhancements

- Enable the remaining soul-base crates (observe, cache, net, tx, blob) once the CLI commands graduate from placeholders.
- Add dedicated tests for strict-mode enforcement and quota logging to catch regressions early.

For additional context see `MIGRATION_GUIDE.md` (migration steps) and `INTEGRATION_SUMMARY.md` (current status and gaps).
