# Observability Stack

This directory bootstraps Prometheus + Grafana so the Browser Use alignment metrics (memory hit rates, gateway throughput, scheduler health) can be inspected locally.

## Prerequisites
- Docker + Docker Compose v2 runtime
- `soulbrowser` running with the metrics server enabled (default `--metrics-port 9090`)

## Usage
```bash
# From repo root
cd observability
docker compose up -d
```

Services:
- Prometheus → http://localhost:9091 (scrapes `host.docker.internal:9090/metrics` every 5s)
- Grafana → http://localhost:3001 (admin/admin by default)
- Alertmanager → http://localhost:9093 (routes alerts to the webhook defined in `alertmanager.yml`)

Grafana auto-loads the dashboards under the **SoulBrowser** folder:

- `browser_use_companion.json` – base dashboard for scheduler/gateway metrics.
- `memory_self_heal.json` – new Stage‑2 panel showing memory hit-rate, template success rate, total queries, and short-term deltas (requires the default `soul_memory_*` gauges to be exported).
- `plugin_helpers.json` – helper registry overview (auto inserts, prompt surfacing, top helpers) powered by `soul_plugin_helper_*` counters.
- Registry overview panels can be added by graphing `soul_plugin_registry_total`, `soul_plugin_registry_active`, and `soul_plugin_registry_pending`; `soul_plugin_registry_last_reviewed_ts` exposes the latest audit timestamp for alerting on stale reviews.
- Helper instrumentation: `soul_plugin_helper_auto_insert_total{plugin,helper}` and `soul_plugin_helper_prompt_total{plugin,helper}` track DSL usage. Use these for dashboards/alerts (e.g., helper flapping or missing prompts) when rolling out new templates; the `plugin_helpers.json` dashboard ships with example queries.

Feel free to tweak/add dashboards by editing the JSON files inside `grafana/dashboards/` and reloading Grafana.

Prometheus now loads alert rules from `rules.yml` and forwards them to Alertmanager, which in turn POSTs to the webhook URL configured inside `alertmanager.yml` (by default `http://host.docker.internal:9300/self-heal-alerts` – point this at whatever Slack/Webhook bridge you prefer).

Sample alerts:

- `MemoryHitRateLow` when `soul_memory_hit_rate_percent` stays below 50% for 5 minutes.
- `SelfHealAutoRetrySpike` when `soul_self_heal_auto_retry_total` increases by more than 5 over 5 minutes.
- `WatchdogEventSpike` when `soul_agent_watchdog_events_total` jumps by more than three events of the same `kind` within 5 minutes (Alertmanager relays these to `/self-heal-alerts` for automation.do).

Hook these alerts to an Alertmanager/Webhook stack to integrate with your existing incident workflow.

Stop everything with `docker compose down` (add `-v` to drop local Prometheus data).
