# Task Stream Protocol

The `/api/tasks/:task_id/stream` WebSocket surfaces everything the planner/runtime knows about a task while it executes. The same feed is now exposed through the Gateway as `/v1/tasks/:task_id/stream`, so remote SDKs can attach without hitting the internal `serve` port.

## Connection
- URL (`serve` WebSocket): `ws://<host>:<port>/api/tasks/<task_id>/stream`
- URL (`serve` SSE): `https://<host>:<port>/api/tasks/<task_id>/events`
- URL (Gateway WebSocket): `ws://<gateway-host>/v1/tasks/<task_id>/stream`
- Authentication: inherits the HTTP auth policy (Serve uses `--auth-token`/`--allow-ip`, Gateway uses adapter token/IP guard).
- Message format: UTF-8 JSON frame (`ws`) or SSE event payload (`events`).

## Event Types
Each payload contains an `"event"` discriminator:

| event            | payload body                                    |
|------------------|-------------------------------------------------|
| `status`         | `status`: snapshot containing task title/status, plan overlays, evidence, etc. |
| `log`            | `log`: `{ cursor, timestamp, level, message }`   |
| `context`        | `context`: latest serialized context snapshot    |
| `observation`    | flattened [`ObservationPayload`](#observationpayload) – realtime evidence & screenshots |
| `overlay`        | `{ "overlay": OverlayPayload }` – plan/execution overlays for the companion UI |
| `annotation`     | `{ "annotation": TaskAnnotation }` – manual or automated notes |
| `watchdog`       | `{ "watchdog": WatchdogEvent }` – planner watchdog detections |
| `judge`          | `{ "verdict": TaskJudgeVerdict }` – judge decisions |
| `self_heal`      | `{ "self_heal": SelfHealEvent }` – auto-heal retries |
| `alert`          | `{ "alert": TaskAlert }` – critical alerts mirrored to webhooks |

### `ObservationPayload`
```json
{
  "event": "observation",
  "observation_type": "image" | "artifact",
  "task_id": "...",
  "step_id": "step-2",
  "dispatch_label": "navigate",
  "dispatch_index": 3,
  "screenshot_path": "soulbrowser-output/.../screen.png",
  "bbox": { "x": 100, "y": 50, ... },
  "content_type": "image/png",
  "recorded_at": "2025-10-21T08:11:00Z",
  "artifact": { "path": "/...", "data_base64": "..." }
}
```
- `observation_type` is derived from the artifact content-type (`image/*` → `image`).
- `artifact` mirrors the persisted artifact entry so clients can download or inline display it.

### `OverlayPayload`
```json
{
  "event": "overlay",
  "overlay": {
    "task_id": "...",
    "source": "plan" | "execution",
    "recorded_at": "2025-10-21T08:11:00Z",
    "data": {
      "step_id": "plan-step-1",
      "title": "Locate checkout button",
      "bbox": { "x": 20, "y": 500, "width": 200, "height": 48 },
      "annotation": "click target"
    }
  }
}
```
Plan overlays describe the planner’s intent; execution overlays describe what the runtime just highlighted/clicked. Both are broadcast in realtime and re-emitted when a client first connects.

### `TaskAnnotation`
Annotations capture self-heal notes, operator comments, or auto-generated warnings:
```json
{
  "event": "annotation",
  "annotation": {
    "id": "...",
    "step_id": "step-3",
    "dispatch_label": "fill-form",
    "note": "Retrying with alternate selector",
    "bbox": { ... },
    "author": "gateway",
    "severity": "warn",
    "created_at": "2025-10-21T08:12:33Z"
  }
}
```

## Recommended Client Flow
1. Create or resume a task via `/api/tasks`/`/v1/tools/run` and capture the `task_id`.
2. Prefer the SSE endpoint (`/api/tasks/:id/events`) so you can rely on `Last-Event-ID` for automatic replay; fall back to the WebSocket for legacy clients.
3. When using SSE:
   - Persist the last `Event.id` you processed and pass it via the `Last-Event-ID` header when reconnecting.
   - Each event ID increments monotonically; gaps mean you missed data and the server will re-stream those events from its ring buffer (currently 256 per task).
4. Handle event payloads as before (status snapshots, observations, overlays, annotations, watchdogs, etc.).
5. Close the stream when `status.status` transitions to `success` or `failed`.

See `web-console/src/components/tasks/TasksPage.tsx` for a reference React client and `sdk/python/README.md` for a minimal Python task stream iterator.
