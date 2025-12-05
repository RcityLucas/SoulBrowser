# Phase 4 Metrics & Telemetry

SoulBrowser exposes Prometheus metrics via the CLI/server so we can build BrowserUse-style dashboards.

## Metrics Endpoint

Run the API server with a metrics port (non-zero) to enable `/metrics`:

```bash
soulbrowser serve --port 8801 --metrics-port 9300
```

Key counters/gauges include:

- `soul_agent_consent_handled_total` – auto-dismissed consent banners
- `soul_agent_fallback_to_baidu_total` – search replans triggered by blockers
- `soul_agent_parser_failures_total` – structured parser/validator errors
- `soul_agent_permission_prompt_total` – permission prompts detected by watchdogs
- `soul_agent_download_prompt_total` – download dialogs detected
- `soul_agent_judge_rejection_total` – Judge/QA rejections (schema mismatch)

Import `grafana_agent_dashboard.json` into Grafana to visualize blocker activity, judge rejections, and parser health.

## Alerts

The dashboard ships with Prometheus expressions such as:

```promql
rate(soul_agent_permission_prompt_total[5m]) > 5
rate(soul_agent_judge_rejection_total[15m]) > 0
```

Use these to wire PagerDuty/SLA alerts whenever the agent needs intervention.

## Registry Authoring Tooling

`soulbrowser registry scaffold --description "Accept site consent" --owner ops --scopes https://example.com --status pending <id>` prints a JSON snippet you can paste into `config/plugins/registry.json`. Each record includes `id`, `status`, `owner`, `description`, and `scopes` so BrowserUse-style registry management is easy to automate.

## K8s/CI Alerts

Add `docs/monitoring/alerts.yaml` via Helm or `kubectl apply`:

```bash
kubectl apply -f docs/monitoring/alerts.yaml
```

Ensure your deployment exposes `/metrics` by setting `--metrics-port` and include the Prometheus scrape config. For CI (GitHub Actions), run a periodic job using `curl` + PromQL check, e.g.:

```yaml
- name: Check judge rejections
  run: |
    VALUE=$(curl -s http://prometheus/api/v1/query --data-urlencode 'query=increase(soul_agent_judge_rejection_total[1h])')
    python ci/check_metric.py "$VALUE" soul_agent_judge_rejection_total
```

`ci/check_metric.py` (example) fails the build if the metric is > 0.
