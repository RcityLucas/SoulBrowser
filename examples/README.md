# Examples Overview

This repository keeps two actively maintained example tracks:

- **Automation DSL samples** (`examples/automation/`)
  - `parallel_sample.dsl` is the canonical Stage-2 regression. Run it via
    `soulbrowser run --script examples/automation/parallel_sample.dsl`.
  - Add new DSL snippets here and wire them into the parser tests so they are
    validated by CI.
- **SDK demonstrations** (`examples/sdk/`)
  - `examples/sdk/README.md` lists the TypeScript and Python scripts that use the
    published SDKs to call `/api/chat`, `/api/tasks/*`, and streaming APIs.
  - These examples stay in sync with the Serve/API plan and are the recommended
    starting point for automation or dashboard integrations.

The older Rust demos (`cargo run --example ...`) and ad-hoc helper scripts are no
longer part of the default toolkit. They remain in the repo for posterity but are
**not** exercised in CI or release builds. See `docs/examples/legacy_examples.md`
for a catalog of those artifacts, their original purpose, and migration notes.
