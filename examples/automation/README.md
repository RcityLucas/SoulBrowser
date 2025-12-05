# Automation DSL Examples

This directory stores ready-to-run DSL scripts used by `soulbrowser run`. Each script
is intentionally small and self-contained so we can use it for Stage-2 regression and
demos without requiring external infrastructure.

## `parallel_sample.dsl`

Demonstrates the Stage-2 parallel block: two branches are executed concurrently with a
local limit of 2 while navigating `example.com` and `iana.org`.

### Running with the CLI

```bash
soulbrowser run \
  --script examples/automation/parallel_sample.dsl \
  --browser chromium \
  --parallel 2 \
  --headless
```

The script:
- Sets two variables (`home`, `docs`).
- Navigates to `https://example.com`.
- Launches a `parallel` block with two `branch` sections.
- Captures a screenshot once the branches finish.

When run with `--parallel 2`, the global semaphore lets both branches execute
concurrently, satisfying the Stage-2 acceptance criteria.

### Entering regression

`parallel_sample.dsl` is referenced by unit tests (see `automation::tests::parallel_sample_parses`)
so parser regressions or missing keywords are caught in CI. Keep any future DSL samples
under `examples/automation/` and add corresponding tests to `src/automation/mod.rs`.
