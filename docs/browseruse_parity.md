# BrowserUse Parity Notes

This document describes how SoulBrowser mirrors BrowserUse's deterministic "对话→搜索→点击权威站→验证→解析→交付" pipeline.

## Deterministic Stage Graph

Informational intents use the stage graph in `config/planner/stage_graph.yaml`, which enforces Navigate → Act → Observe → Validate → Parse → Deliver. `StageAuditor` repairs any missing stage by inserting:

- `browser.search` during Navigate (with guardrail-derived query/site hints)
- `AutoActStrategy` (search submission + authority click) during Act
- `data.extract-site` during Observe
- `data.validate-target` during Validate
- `data.parse.*` + `data.deliver.structured` during Parse/Deliver

## Message Manager Parity

`crates/soulbrowser-kernel/src/agent/message_manager.rs` reimplements BrowserUse's MessageManager. CLI and Serve flows now:

- maintain `<initial_user_request>`, `<follow_up_user_request>`, `<read_state_x>` snippets
- feed `agent_task_prompt`/`agent_history_prompt` to LLM planners and replanners
- serialize the full message state into API/CLI outputs (`message_state` field and `message_state.json`)

## Search + Guardrails

`AutoActStrategy` recognizes Baidu, Google, and Bing result pages. It automatically types the query, submits the search, waits for result panes, and clicks the first authority hit with `derive_guardrail_domains()` validation. Guardrail keywords feed search terms via `StageContext`, so deterministic replans reuse authority-specific queries.

### Telemetry

- `soul_guardrail_keyword_seeds_total` increments whenever guardrail keywords extend the search terms for an intent, exposing how often the deterministic loop leans on guardrails.
- `soul_auto_act_search_engine_total` tracks which search engine (Baidu/Google/Bing) the AutoAct strategy serviced so we can monitor coverage and detect failures.
- `message_state.json` artifacts and the new `message_state` task-stream event let the Serve UI display BrowserUse-style Evaluate/NextGoal panes without recomputing prompts.
  - The live session stream (`/api/sessions/{id}/live`) now emits `message_state` events so Serve dashboards mirror BrowserUse's Evaluate/NextGoal/Memory panels in real time.
  - `/api/tasks/<task_id>/message_state` exposes the latest snapshot (live if execution is still running, otherwise from `message_state.json`) so Serve/CLI consoles can hydrate their panels without scraping plan/exec logs.
  - `soul_cli console --input <run.json>` now forwards the `message_state` blob inside its `/data` payload, enabling BrowserUse-style Evaluate/NextGoal cards inside the lightweight console preview, and when the preview is opened against a Serve task it subscribes to `/api/tasks/<task_id>/events` for streaming `message_state` updates.
  - Plan overlays emit `guardrail_keywords` and `auto_act_engine` badges, and those overlays are mirrored to the Serve/CLI timeline via annotations so AutoAct/guardrail telemetry appears as live badges when the plan executes.

## Validation/Deliver Tests

`agent::tests::informational_pipeline_includes_observe_validate_and_deliver` asserts the repaired plan includes `data.extract-site`, `data.validate-target`, parser, and `data.deliver.structured` steps in order when guardrail hints are present.

## Persisted Artifacts

`persist_execution_outputs` now writes `message_state.json` next to plan/execution logs, mirroring BrowserUse's timeline telemetry. CLI structured output also exposes the state for UI rendering.

Refer to `dplan/browseruse_alignment_plan.md` for remaining workstreams (UI overlays, demo scripts, telemetry). A CLI walkthrough lives in `docs/demos/browseruse_loop.md`.
