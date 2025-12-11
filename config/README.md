# Config Directory

Active configuration samples live here. Copy the `.example` files to environment-
specific names and keep secrets out of version control.

| File/Dir | Purpose | Notes |
| --- | --- | --- |
| `config.yaml.example` | Top-level CLI/runtime defaults. | Copy to `config.yaml` (ignored) when you need to override defaults. |
| `local.env.example` | Environment variables consumed by `dotenvy`. | Duplicate as `local.env` for local testing; never commit the populated file. |
| `data_extract_profiles.example.json` | Chrome profile allowlist for perception runs. | Copy to `data_extract_profiles.json` only if you need custom profile hints. |
| `permissions/` | Permission policy bundles consumed by the gateway. | Keep policies per tenant; add subfolders here. |
| `plugins/` | Plugin registry + manifests. | Default registry lives under `plugins/registry.example.json`; copy to `registry.json` when enabling plugins. |
| `policies/` | Execution policies for gateway/serve. | Populate with org-specific policy files. |
| `self_heal.yaml` | Default self-heal strategy configuration. | Treated as a sample; copy to `tenants/<tenant>/self_heal.yaml` for overrides. |

Legacy/experimental configs should move to `config/archive/` with a short README
explaining replacement steps. This keeps the root directory focused on active
`.example` sources, per the Project Slimming plan.
