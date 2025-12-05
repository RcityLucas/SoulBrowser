# Self-Heal Strategies

Stage-2 introduces a configurable registry of self-heal strategies. Each strategy lives in
`config/self_heal.yaml` and controls how the runtime reacts to failures (auto-retry, human
approval, annotations).

## Configuration

```yaml
strategies:
  - id: auto_retry
    description: Automatically retry failed dispatches before surfacing an error.
    enabled: true
    tags: [stability, retry]
    telemetry_label: auto_retry
    action:
      type: auto_retry
      extra_attempts: 1
  - id: human_confirmation
    description: Escalate terminal failures for human confirmation.
    enabled: false
    tags: [escalation, approval]
    telemetry_label: human_confirm
    action:
      type: human_approval
      severity: warn
```

When SoulBrowser starts it loads the YAML (falling back to built-in defaults). Modifying the
file and restarting updates the applied strategies; CLI/API toggles persist the file directly.

## CLI

```
soulbrowser self-heal list
soulbrowser self-heal list --json
soulbrowser self-heal enable auto_retry
soulbrowser self-heal disable human_confirmation
```

Each retry triggered by `auto_retry` appears in the task log and as a real-time annotation on
the task stream.

## HTTP API

```
GET  /api/self-heal/strategies
POST /api/self-heal/strategies/{strategy_id} {"enabled":true|false}
```

These endpoints power the Diagnostics > Self-Heal controls in the web console. The response
shape matches the YAML schema so frontends can render descriptions/status directly.
