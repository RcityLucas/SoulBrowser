# Legacy Examples and Scripts

The artifacts below remain in the tree for historical reference but are no longer
maintained. Treat them as inspiration or pseudocode; do not rely on them for
production workflows.

## Rust demonstration binaries

| Path | Original intent | Legacy status |
| ---- | --------------- | ------------- |
| `docs/examples/legacy_code/basic_demo.rs` | Conceptual "build a Soul" walkthrough with a fake CDP adapter, state center, and locator. | Stubs every subsystem; never talks to the real Serve stack or Chrome. Use the DSL/SDK samples instead. |
| `docs/examples/legacy_code/standalone_demo.rs` | Alternate architecture mock showing scheduling/memory concepts without dependencies. | Mirrors the basic demo and predates the current layered modules; kept only as educational prose. |
| `docs/examples/legacy_code/basic_navigation.rs` | `cargo run --example basic_navigation` script that prints pretend navigation events. | Uses hard-coded sleeps/logs and does not exercise any runtime components. |
| `docs/examples/legacy_code/form_automation.rs` | Illustrates a multi-step form flow with mocked selectors and delays. | Same limitations as the navigation demo; no CDP wiring, no Serve APIs. |
| `docs/examples/legacy_code/advanced_tools.rs` | Narrative example of “advanced tools” (data extract, structured output). | Relies on outdated tool traits and is not compiled during CI. |
| `docs/examples/legacy_code/soul_integration_demo.rs` | Showcased integration with `soul-base` components (auth/interceptors). | References the now-archived soul-base adapters; kept for reference while the new gateways mature. |
| `docs/examples/legacy_code/soul_reuse_example.rs` | Demonstrated reusing soul-base modules directly inside SoulBrowser. | Obsolete after the Serve/API restructuring; left here for context when reading older design docs. |

## Helper scripts

| Path | Purpose | Legacy status |
| ---- | ------- | ------------- |
| `scripts/run_visual_console.sh` | Built + launched `soulbrowser serve` with a handful of env tweaks. | Superseded by directly running `cargo run --bin soulbrowser -- serve`; script is untested and will drift when new flags appear. |
| `scripts/start_web_console.sh` | Spawned the backend binary and the Vite dev server in one shot. | Duplicates the steps already documented in `docs/guides/WEB_CONSOLE_USAGE.md`; the script still works for simple demos but is no longer updated. |
| `scripts/soak_test.sh` | Hammered `/api/chat` in a loop and scraped Prometheus counters. | Uses legacy metric names (`soul_memory_hit_rate_percent`, etc.) and has no retry/auth handling. Keep it only as a template for custom load tests. |
| `scripts/mint_with_royalty.py` | Convenience helper for the historical NFT/royalty showcase. | Relies on the deprecated “mint with royalty” flow and external services; not part of the supported demo set. |

## Legacy integration tests

The previous test suite exposed several “full-stack” benches that depended on the old
soul-base adapter stack. They have been archived alongside the demos so the default
workspace stays lean:

| File (`docs/examples/legacy_code/tests/…`) | Notes |
| --- | --- |
| `e2e_test.rs` | Early end-to-end harness that wired directly into the soul-base adapters. |
| `integration_test.rs` | Exercised obsolete L3/L4 APIs that no longer exist in the Serve stack. |
| `stress_test.rs` | Simple `/api/chat` stress loop that assumed the removed `full-stack` feature gate. |
| `soul_base_integration_test.rs` | Verified the legacy soul-base wiring; Serve/AppContext tests have replaced its coverage. |

When in doubt, prefer the maintained examples listed in `examples/README.md`. The
legacy entries above should not block refactors—feel free to delete or move them to
`ARCHIVE/` once they are no longer useful for storytelling.
