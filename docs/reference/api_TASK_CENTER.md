# Task Center API Reference

The Task Center exposes REST endpoints for managing agent tasks plus a streaming channel for real-time updates. Both the Web Console and SDKs rely on these contracts; keep them additive and backward compatible.

Base URL: `http://<host>:<port>/api`
Authentication: development builds run without auth. In hardened deployments, attach `Authorization: Bearer <token>` via the gateway adapter.

---

## 1. Create a task — `POST /api/chat`

Request body:
```jsonc
{
  "prompt": "在 https://www.wikipedia.org 搜索 SoulBrowser",
  "planner": "llm",              // or "rule-based"
  "model": "gpt-4o-mini",        // optional override when planner=llm
  "execute": true,                // false = planning only
  "headful": false,               // forwarded to runtime
  "memory_template": "templates:checkout-flow", // optional
  "metadata": {"tags": ["demo", "wiki"]}
}
```

Response:
```json
{
  "task_id": "tsk_01HXY3...",
  "status": "queued",
  "plan": { "steps": [...] },
  "links": {
    "task": "/api/tasks/tsk_01HXY3...",
    "stream": "/api/tasks/tsk_01HXY3.../stream"
  }
}
```

Notes:
- `plan` follows `docs/reference/PLAN_SCHEMA.md` (UI renders each `steps[*]`).
- Open `links.stream` immediately for realtime updates.

---

## 2. List tasks — `GET /api/tasks`

Query params:
- `limit` (default 20, max 100)
- `status` (`queued|running|success|failed`)
- `since` (RFC3339 timestamp for incremental polling)
- `order` (`desc` default)

Response:
```json
{
  "data": [
    {
      "task_id": "tsk_01HXY3...",
      "status": "running",
      "created_at": "2025-03-10T08:51:32Z",
      "plan_summary": "搜索 SoulBrowser",
      "last_event_at": "2025-03-10T08:52:10Z"
    }
  ],
  "next_cursor": "tsk_01HXY1..."   // omit when no more data
}
```

---

## 3. Inspect a task — `GET /api/tasks/{task_id}`

Response payload:
```jsonc
{
  "task_id": "tsk_01HXY3...",
  "status": "success",
  "created_at": "2025-03-10T08:51:32Z",
  "updated_at": "2025-03-10T08:53:02Z",
  "prompt": "在 https://www.wikipedia.org 搜索 SoulBrowser",
  "planner": {
    "name": "llm",
    "model": "gpt-4o-mini",
    "retries": 0
  },
  "plan": { ... full schema ... },
  "plan_overlays": {
    "steps": [
      {"id": "step-1", "bbox": [0,0,640,80], "recorded_at": "2025-03-10T08:51:40Z"}
    ]
  },
  "execution_overlays": [
    {
      "type": "highlight",
      "bbox": [120, 340, 360, 420],
      "recorded_at": "2025-03-10T08:52:01Z",
      "meta": {"action": "click", "selector": "#searchInput"}
    }
  ],
  "artifacts": {
    "count": 2,
    "items": [
      {"label": "screenshot", "path": "soulbrowser-output/artifacts/tsk_01HXY3.../wiki.png"}
    ]
  },
  "logs": [
    {"level": "info", "message": "Navigated to https://www.wikipedia.org", "recorded_at": "2025-03-10T08:51:45Z"}
  ]
}
```

Overlay JSON originates from `src/visualization.rs`; always include `recorded_at` so the UI can sort events.

---

## 4. Stream events — `GET /api/tasks/{task_id}/stream`

Protocol: Server-Sent Events (SSE). Callers should set `Accept: text/event-stream` and keep the HTTP connection open. (Gateway adapters expose the same payloads via WebSockets.)

Event types:

| `event` | Description |
|---------|-------------|
| `status` | Snapshot containing `status`, `current_step`, and latest `plan_overlays`. |
| `log` | Structured runtime log with `level`, `message`, `recorded_at`. |
| `overlay` | Realtime visual overlay. Fields: `type`, `bbox`, `content_type`, `recorded_at`. |
| `artifact` | Fired when a file/artifact is written. Includes `label`, `path`, optional `mime`. |
| `error` | Terminal failure with `error_code`, `message`, `hint`. |

Example stream:
```text
event: status
data: {"status":"running","current_step":2,"plan_overlays":{"steps":[...]}}

event: log
data: {"level":"info","message":"Clicked search button","recorded_at":"2025-03-10T08:52:00Z"}

event: overlay
data: {"type":"highlight","bbox":[120,340,360,420],"content_type":"dom","recorded_at":"2025-03-10T08:52:01Z"}
```

Use the `Last-Event-ID` header to resume after network interruptions.

---

## 5. Related references
- Planner schema & overlays: `docs/reference/PLAN_SCHEMA.md`
- Web console usage: `docs/guides/WEB_CONSOLE_USAGE.md`
- SDK type definitions: `sdk/python/soulbrowser_sdk/types.py`, `sdk/typescript/src/types.ts`

Update this document whenever the payload changes so SDK/UI teams stay in sync.
