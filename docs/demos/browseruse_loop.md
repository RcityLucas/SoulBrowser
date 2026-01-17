# BrowserUse-style Informational Loop Demo

This walkthrough exercises the deterministic informational pipeline (search → auto-act → observe → validate → parse → deliver) and shows where telemetry/message-state artifacts land.

## Prerequisites

1. Launch the local browser runtime (Serve or CLI executor) so `soul_cli chat --execute` can control a session.
2. Ensure Prometheus metrics are exposed (e.g., run `SOUL_METRICS_ADDR=0.0.0.0:9090 cargo run --bin soul_serve`) so the new counters are visible.

## Run the Informational Task

```bash
scripts/browseruse_demo.sh "我想看下现在办理最多的案件是那种" soulbrowser-output/demo
```

The script wraps `soul_cli chat --execute` and saves `artifacts.json` + `run.json` under the provided output directory so you can replay the plan/execution later.

What to expect:

- The generated plan contains `browser.search`, AutoAct steps (focus → type → submit → guardrail click), `data.extract-site`, `data.validate-target`, `data.parse.generic`, and `data.deliver.structured` in stage order.
- Execution artifacts are written under `soulbrowser-output/tenants/.../tasks/<task-id>/`, including the new `message_state.json` file and `telemetry.json` snapshot.
- CLI structured output includes a `message_state` blob mirroring BrowserUse's MessageManager panels.

## Inspect Telemetry

Open your metrics endpoint (default `/metrics`) and look for:

- `soul_guardrail_keyword_seeds_total{intent="informational"}` – increments when guardrail keywords seed search terms.
- `soul_auto_act_search_engine_total{intent="informational",engine="baidu"}` (or `bing`/`google`) – increments whenever AutoAct submits/reties the respective search.

These counters confirm the deterministic guardrail/search behavior even if the upstream LLM plan omits it.

## Visualize Message State

Serve's task stream now emits a `message_state` event, and `message_state.json` is persisted next to executions. UI overlays can replay Evaluate/NextGoal/Memory sections directly from this data, keeping parity with BrowserUse's timeline.

To inspect the snapshot without replaying the full run bundle, call the new Serve endpoint:

```bash
curl http://localhost:8787/api/tasks/<task-id>/message_state | jq
```

or load the saved run bundle in the lightweight console preview:

```bash
soul_cli console --input soulbrowser-output/demo/run.json --serve
```

The `/data` payload now includes the `message_state` blob so BrowserUse-style Evaluate / NextGoal / Memory panels can be rendered directly in the console UI or any custom dashboard.

Use this demo to capture GIFs/logs for parity reviews and to troubleshoot guardrail-driven replans.

You can diff the saved `plan.json`/`executions.json` to ensure AutoAct steps include `expected_url` metadata pointing to the authority domain and that the Serve UI displays the Evaluate/NextGoal panels via `message_state` events.
