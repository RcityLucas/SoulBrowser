# Planner Custom Tool Reference

This document tracks the exact custom tool identifiers that the agent planner and
`PlanValidator` accept. Plans that reference a tool outside this list will be
rejected before execution, so plan authors should treat this as the source of
truth together with `config/planner/custom_tool_allowlist.json`.

## Allowed identifiers

| Tool id | Stage | Notes |
| --- | --- | --- |
| `data.extract-site` | Observe | Primary DOM/AX snapshot tool for any structured workflow. |
| `page.observe` | Observe | Legacy alias honoured by the validator (new plans should prefer `data.extract-site`). |
| `data.parse.generic` | Parse | Fallback parser for unstructured observations. |
| `data.parse.market_info` | Parse | Emits `market_info_v1` payload. |
| `data.parse.news_brief` | Parse | Emits `news_brief_v1` payload. |
| `data.parse.weather` | Parse | Emits `weather_report_v1` payload. |
| `data.parse.twitter-feed` | Parse | Emits `twitter_feed_v1` payload. |
| `data.parse.facebook-feed` | Parse | Emits `facebook_feed_v1` payload. |
| `data.parse.hackernews-feed` | Parse | Emits `hackernews_feed_v1` payload. |
| `data.parse.linkedin-profile` | Parse | Emits `linkedin_profile_v1` payload. |
| `data.parse.github-repo` | Observe + Parse | GitHub account/repo parser. `payload.username` is required. |
| `github.extract-repo` / `data.parse.github.extract-repo` | Observe + Parse | Historical aliases kept for compatibility. |
| `weather.search` | Navigate | Macro helper that opens weather search results and waits for the widget. |
| `data.deliver.structured` | Deliver | Canonical structured output delivery. Supply `schema`, `artifact_label`, and `source_step_id`. |
| `agent.note` | Deliver | Inline reporting step (also catches names ending with `note`). |
| `plugin.*` | Custom | Plugin bridge namespace; ensure the plugin id exists. |
| `mock.llm.plan` | Test | Stub used by mock planner flows. |

Additional parser aliases (for example `data.parse.twitter_feed`) are captured in
`config/planner/custom_tool_allowlist.json` so the validator and lint tooling can
flag them as legacy or allow them in migrations.

## Plan linting

`./scripts/ci/lint_plan_tools.py` reads the allowlist JSON above and checks every
`plan*.json` file in the repo (or explicit file arguments). It reports unsupported
custom tool ids before CI reaches the kernel validator. The script now runs as
part of `scripts/dev_checks.sh`, so contributors will see a failure locally if a
plan references non-canonical tools such as `deliver`.

## Workflow reminder

Structured tasks must still follow the `navigate -> data.extract-site ->
data.parse.* -> data.deliver.structured` progression described in
`docs/legacy/reference/PLAN_SCHEMA.md`. Use this document together with the
schema catalog to pick the right parser + deliver pair for each plan.
