# soulbrowser-kernel Design Notes

## Goals

1. Extract the shared runtime (scheduler, registry, perceivers, policy center, tool registry, storage) into a dedicated `soulbrowser-kernel` crate that can be reused by Serve/Demo/Gateway binaries.
2. Reduce the size of `src/main.rs` by moving lifecycle wiring and helper routines behind a typed kernel API.
3. Enable future alternative frontends (HTTP/gRPC/gateway/web-console) to link against the same kernel without copying CLI-only glue.

## Layer Mapping

| Layer | Existing Modules | Target home |
|-------|------------------|-------------|
| L0 runtime & adapters | `cdp_adapter`, `permissions-broker`, `network-tap-light`, `l0_bridge.rs` | `soulbrowser-kernel::runtime` |
| L1 state & scheduling | `registry`, `scheduler`, `state-center`, `app_context.rs` | `soulbrowser-kernel::state` |
| L2 perception | `perceiver-*`, `perception_service.rs` | `soulbrowser-kernel::perception` |
| L3 actions | `agent`, `automation`, `tools.rs`, `task_store.rs` | `soulbrowser-kernel::actions` |
| L4 persistence | `soulbrowser-event-store`, `storage.rs`, `structured_output.rs` | `soulbrowser-kernel::persistence` |
| L5 tools | `src/tools.rs`, `scripts/page_observe.js` | `soulbrowser-kernel::tools` |
| L6 governance | `metrics.rs`, `gateway_policy.rs`, `policy.rs`, `task_status.rs` | `soulbrowser-kernel::governance` |
| L7 surfaces | Serve/Gateway/Demo CLI commands | Thin wrappers that depend on kernel |

## Crate Layout Sketch

```
crates/
  soulbrowser-kernel/
    src/
      lib.rs                 # prelude + Kernel entrypoints
      kernel.rs              # Kernel struct and command lifecycle
      config.rs              # typed AppConfig facade (moved from src/config.rs)
      state/
        mod.rs               # AppContext extraction
      tools/
        mod.rs               # BrowserToolManager + manifest registry
      perception/
        mod.rs               # PerceptionService + adapters
      runtime/
        mod.rs               # CDP bootstrap + permissions bridge
      governance/
        mod.rs               # Policy center facade, task status, rate limiting hooks
```

### API Draft

```rust
pub struct Kernel {
    ctx: Arc<AppContext>,
}

impl Kernel {
    pub async fn initialize(cfg: KernelConfig) -> Result<Self>;
    pub async fn serve(&self, opts: ServeOptions) -> Result<()>;
    pub async fn gateway(&self, opts: GatewayOptions) -> Result<()>;
    pub async fn demo(&self, opts: DemoOptions) -> Result<DemoArtifact>;
}
```

- `KernelConfig` wraps the existing `Config` struct (currently built inside `src/main.rs` and `src/cli/serve.rs:33-140`).  Validation and defaults move into `crates/soulbrowser-kernel/src/config.rs` so subcommands just call `Kernel::initialize`.
- `ServeOptions/GatewayOptions` mirror the flags defined under `src/cli` today. They should remain slim DTOs so future binaries (eg `soulbrowser-gateway`) can reuse them without linking clap.

## Extraction Plan

1. **Bootstrap crate** – add `crates/soulbrowser-kernel` with the module list currently exposed from `src/lib.rs` (agent, app_context, auth, browser_impl, config, errors, intent, interceptors, judge, l0_bridge, llm, metrics, observation, parsers, plugin_registry, policy, replan, self_heal, storage, structured_output, task_status, tools, types, watchdogs). Re-export from the root crate to keep tests stable.
2. **Lift AppContext** – move `src/app_context.rs` into the kernel crate and expose it via `soulbrowser_kernel::state::AppContext`. Update CLI code (`src/cli/serve.rs:63-130`, `src/main.rs:825-987`) to import from the kernel crate instead of local modules.
3. **Move toolchain** – relocate `src/tools.rs` and associated manifest helpers to `soulbrowser-kernel::tools`. Replace the ad-hoc registration loop inside `BrowserToolManager` with declarative manifests (static array + iteration). → **partially done** (`BrowserToolManager` 已迁移至 kernel，但注册逻辑仍需数据驱动优化）。
4. **Expose Kernel facade** – introduce `Kernel` struct plus options as shown above. Start by moving Serve wiring (目前 ~700 LOC 在 `src/main.rs:1320-2050`) into `Kernel::serve` so the binary only handles argument parsing and logging。→ ✅ `Kernel::serve`/`Kernel::gateway` 已实现，下一步是为 demo/replay 等命令增加对应 API。

Each phase should keep the CLI compiling by re-exporting the moved modules in `src/lib.rs` until all callers switch to the new crate.

## Soul-base Integration Adapter

### Background

- `src/browser_impl.rs:6-176` and `src/tools.rs:66-219` interact directly with soul-base crates (`soulbase-auth`, `soulbase-tools`, `soulbase-storage`).
- Relative path dependencies (`../soul-base-main/...`) listed under `Cargo.toml:136-153` require co-locating repositories.
- The `MIGRATION_GUIDE.md` documents soul-base usage but offers no abstraction for consumers who do not vend those crates.

### Proposed crate: `crates/integration-soulbase`

Responsibilities:

1. **Trait definitions** – introduce traits under `soulbrowser-kernel` for auth, storage, and tool registry (eg `AuthProvider`, `ToolRegistryProvider`, `StorageProvider`).
2. **Soul-base adapters** – in the new `integration-soulbase` crate, implement those traits using the real soul-base clients. Example:
   ```rust
   pub struct SoulbaseAuthProvider {
       inner: BrowserAuthManager,
   }
   impl AuthProvider for SoulbaseAuthProvider {
       async fn authenticate(&self, token: String) -> Result<AuthSession> { ... }
   }
   ```
3. **Feature flags** – expose the adapter behind a `soulbase` cargo feature. When disabled, the kernel compiles against lightweight in-memory mocks so the repo builds standalone.
4. **Config wiring** – move all `soulbase`-specific environment toggles from `src/main.rs` into the adapter crate, so the CLI only toggles the feature via cargo flags or runtime config.

### Implementation Checklist

- [ ] Define traits in `soulbrowser-kernel::integration` for auth/storage/tools bridging.
- [ ] Move `BrowserAuthManager`, `SessionManager`, `StorageManager`, `BrowserToolManager` definitions into the kernel crate under an `integration` module.
- [ ] Add `crates/integration-soulbase` with implementations backed by `soulbase-*` crates.
- [ ] Update `Cargo.toml` dependencies to remove the direct relative paths from the root package; only the integration crate should reference them.
- [ ] Expose an opt-in `soulbase` feature on the CLI to pull the adapter (default to the in-memory mocks for local dev).

This separation lets us colocate soul-base-specific code while keeping the kernel consumable and buildable without the sibling repository.

## Next Steps

1. Wire up the newly created `soulbrowser-kernel` crate in the workspace and re-export it from `src/lib.rs`.
2. Move `app_context.rs` + dependencies into that crate and adjust CLI imports accordingly.
3. Introduce the `integration-soulbase` crate scaffold (traits + empty impl) so that the soul-base bridges have a dedicated home.

Tracking these tasks in a checklist (see `docs/kernel_rearchitecture.md`) keeps the refactor incremental and reviewable.
