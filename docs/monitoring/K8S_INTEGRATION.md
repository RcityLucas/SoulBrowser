# K8s Integration

1. Enable `/metrics` in deployment:

```yaml
args:
  - serve
  - --port=8801
  - --metrics-port=9300
```

2. Prometheus scrape config:

```yaml
scrape_configs:
  - job_name: 'soulbrowser-agent'
    static_configs:
      - targets: ['soulbrowser-agent:9300']
```

3. Apply alert rules:

```bash
kubectl apply -f docs/monitoring/alerts.yaml
```

4. CI check example (`.github/workflows/alerts.yml`):

```yaml
- name: Judge rejection budget
  run: |
    RESP=$(curl -s $PROM_URL/api/v1/query --data-urlencode 'query=increase(soul_agent_judge_rejection_total[1h])')
    python ci/check_metric.py "$RESP" soul_agent_judge_rejection_total
```
