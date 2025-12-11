# Documentation Index

Use this index to jump to the right part of the knowledge base and avoid hunting through dozens of Markdown files. Drop any new docs into the matching subdirectory and update this file so future contributors can discover them.

## Agent & Execution
- `agent/` – Planner/executor prompts, execution plans, and agent behavior specs.

## Guides
- `guides/README.md` – Quick index of every hands-on article.
- `guides/BACKEND_USAGE.md`, `guides/FRONTEND_SETUP_GUIDE.md` – Backend/frontend bring-up.
- `guides/PARSER_DEVELOPMENT.md` – Scaffold a new parser + schema (`parser_scaffold` usage, integration steps).
- `guides/TWITTER_FEED.md` – End-to-end example for the Twitter/X parser and structured delivery.
- `guides/TROUBLESHOOTING.md`, `guides/TUTORIAL_OUTLINE.md`, `guides/VISUAL_TESTING_CONSOLE.md`, `guides/WEB_CONSOLE_USAGE.md` – Hands-on walkthroughs and triage notes.

## Plans & Architecture
- `plans/flexible_parser_expansion.md` – Roadmap for expanding structured parsers/tools.
- `plans/serve_api_optimization.md` – End-to-end Serve/API hardening plan (router, context, perception, planner, Task Center).
- `plans/project_slimming.md` – Detailed plan for removing redundant docs/code/deps and simplifying the repo.

## Monitoring & Operations
- `monitoring/README.md` – How we wire metrics, logs, alerts, and dashboards.
- `monitoring/K8S_INTEGRATION.md` – Cluster deployment notes.
- `monitoring/alerts.yaml` / `monitoring/grafana_agent_dashboard.json` – Ready-to-import alerting + dashboard assets.

## Metrics & Reference
- `metrics/` – Dashboards and metric-export instructions.
- `reference/` – API specs, perception payload schemas, security/perf checklists.
  - `reference/schema_catalog.md` – Canonical list of `data.parse.*` schemas (GitHub/Twitter/Facebook/etc.).
  - `reference/schemas/*.json` – JSON Schema definitions consumed by `data.deliver.structured`.

## Examples & Tooling
- `examples/` – Active automation + SDK samples; see `examples/README.md` for the curated list and `docs/examples/legacy_examples.md` for the archived demos.
- `scripts/` – Local maintenance helpers (cleanup, profile reset, dev checks); headers call out support status. Legacy helpers are cataloged alongside the demos in `docs/examples/legacy_examples.md`.

## Archive
- `ARCHIVE/` – Legacy docs that remain for historical reference.
  - `START_SERVER.md` – replaced by `guides/BACKEND_USAGE.md`.
  - `Perceive_API_浏览器问题解决.md` – replaced by `guides/TROUBLESHOOTING.md`.

> ✅ When adding documentation, choose the right folder first, then link it here so the rest of the team can find it quickly.
