# Agent Plan Schema

Agent plans produced by `ChatRunner`/`RuleBasedPlanner` share a stable JSON
contract. This document summarizes the structure consumed by the Web Console,
SDKs, and downstream tooling.

## Plan root (`plan.plan`)

| Field | Type | Description |
| --- | --- | --- |
| `task_id` | `string` | Identifier propagated through execution and Task Center APIs. |
| `title` | `string` | Human-readable subject line. |
| `description` | `string` | Optional longer summary. |
| `created_at` | `RFC3339 timestamp` | Planner emission time. |
| `meta.rationale` | `string[]` | Planner reasoning bullets. |
| `meta.risk_assessment` | `string[]` | Known risks/assumptions. |
| `meta.vendor_context` | `map<string, json>` | Provider-specific metadata (`planner_name`, prompt ids, etc.). |
| `steps` | `AgentPlanStep[]` | Ordered execution steps (see below). |

### `AgentPlanStep`

| Field | Type | Description |
| --- | --- | --- |
| `id` | `string` | Unique within the plan (`step-1`, `step-2`, …). |
| `title` | `string` | Short action verb ("Navigate to…"). |
| `detail` | `string` | Expanded instruction/resolution steps. |
| `tool` | `AgentTool` | Action payload/timeout/wait hints. |
| `validations` | `AgentValidation[]` | Optional post-action checks. |
| `requires_approval` | `bool` | Whether operator approval is required before executing. |
| `metadata` | `map<string, json>` | Planner-provided hints (locators, anchor ids, annotations). |

### `AgentTool`

```jsonc
{
  "kind": {
    "Navigate": {"url": "https://example.com"}
    // or "Click", "TypeText", "Select", "Scroll", "Wait", "Custom"
  },
  "wait": "dom_ready" | "idle" | "none",
  "timeout_ms": 20000 // optional per-step timeout
}
```

Custom tool payloads follow the parser/delivery tool specs (`data.extract-site`,
`data.parse.*`, `data.deliver.structured`, `agent.note`, etc.). See
`docs/reference/schema_catalog.md` for structured output schemas.

### `AgentValidation`

```jsonc
{
  "description": "Confirm search results visible",
  "condition": {
    "ElementVisible": {"Css": "#results"}
    // or ElementHidden, UrlMatches, TitleMatches, NetworkIdle, Duration
  }
}
```

## Flow overlays (`plan.overlays` / `flow.execution.overlays`)

Planner overlays highlight intended DOM targets using the metadata collected in
`AgentPlanStep.metadata`. Execution overlays are emitted in realtime via
Task Center streams once screenshots/artifacts become available. Both payloads
share the structure defined in `src/visualization.rs` (`recorded_at`, `bbox`,
`data`, `route`).

## References
- Task Center API: `docs/reference/api_TASK_CENTER.md`
- Structured parsers/delivery: `docs/reference/schema_catalog.md`
- SDK types: `sdk/python/soulbrowser_sdk/types.py`, `sdk/typescript/src/types.ts`
