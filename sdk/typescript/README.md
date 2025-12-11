# SoulBrowser TypeScript SDK (preview)

Early TypeScript bindings for the SoulBrowser testing console APIs. The SDK wraps the
HTTP endpoints exposed by `soulbrowser serve` so scripts and tools can drive agent
planning, task execution, and status streams without reimplementing request plumbing.

> **Status**: preview. The API surface will evolve as Stage-1/Stage-3 tasks land.

## Installation

```
npm install soulbrowser-sdk
# or
pnpm add soulbrowser-sdk
```

> Node.js 18+ is required. If you target older runtimes, provide a `fetch` polyfill
> and a WebSocket constructor when instantiating the client.

## Quick start

```ts
import { SoulBrowserClient } from 'soulbrowser-sdk';

const client = new SoulBrowserClient({
  baseUrl: 'http://127.0.0.1:8801',
  // Optional: pass custom fetch / WebSocket implementations
  // fetchFn: (input, init) => fetch(input, init),
  // WebSocketClass: WebSocket,
});

const chat = await client.chat({
  prompt: 'open example.com and capture a screenshot',
  execute: false,
  capture_context: true,
  current_url: 'https://example.com'
});

if (chat.context?.screenshot_base64) {
  console.log('Context screenshot size', chat.context.screenshot_base64.length);
}
```

## API surface (initial)

| Method | Description |
| ------ | ----------- |
| `chat(request)` | Calls `/api/chat` and returns the structured plan/flow/context payload. |
| `createTask(request)` | Creates (and optionally executes) a task via `/api/tasks`. |
| `getTask(taskId, limit?)` | Fetches task detail (summary + dispatches). |
| `listTasks(limit?)` | Returns the summarized task list. |
| `getTaskStatus(taskId)` | Returns the live status snapshot. |
| `getTaskLogs(taskId, since?)` | Streams accumulated log entries once. |
| `getTaskArtifacts(taskId)` | Lists artifacts + summary co-located with the task. |
| `openTaskStream(taskId)` | Opens a WebSocket to `/api/tasks/:id/stream`; relies on browser `WebSocket` or the injected constructor. |
| `streamTaskEvents(taskId, opts?)` | Consumes the `/api/tasks/:id/events` SSE feed with automatic reconnection + `Last-Event-ID` replay. |

Additional endpoints (`/api/perceive`, `/api/tasks/:id/execute`, etc.) will be added
as the orchestration plan progresses.

### Gateway HTTP client (Stage-1 parity)

The SDK now exposes `SoulBrowserGatewayClient` for talking to the L7 Adapter / Gateway
(`/v1/tools/run`). This is how Browser Use-compatible SDKs trigger individual tools
or action flows from remote services.

```ts
import { SoulBrowserGatewayClient } from 'soulbrowser-sdk';

const gateway = new SoulBrowserGatewayClient({
  baseUrl: 'http://127.0.0.1:8710',
  tenantId: 'demo-tenant',
  apiKey: process.env.SOULBROWSER_GATEWAY_TOKEN,
});

const response = await gateway.runTool({
  tool: 'navigate',
  params: { url: 'https://example.com' },
  timeout_ms: 20_000,
});

console.log('Gateway status', response.status, response.trace_id);
```

Headers can be overridden per-call (`gateway.runTool(payload, { tenantId, headers })`) to
support multi-tenant orchestrators or custom auth shims.

#### Gateway plan demo script

To exercise the new `/v1/tasks/run` endpoint (which executes an entire agent plan and
streams overlays/logs), a runnable sample lives in `examples/gateway-plan-demo.ts`.

```
cd sdk/typescript
npm install
npx ts-node --esm examples/gateway-plan-demo.ts --gateway http://127.0.0.1:8710 --tenant demo-tenant
```

The script posts a minimal plan (navigate to https://example.com and wait for the
title) and automatically opens a WebSocket to `/v1/tasks/{task_id}/stream`, printing
status/overlay events until the plan succeeds. Provide `GATEWAY_TOKEN` via env if your
Gateway policy requires bearer tokens.

### Working with plans, overlays, and artifacts

```ts
const plan = await client.chat({ prompt: '检查 SoulBrowser docs', execute: false });
plan.plan?.plan?.steps?.forEach((step) => {
  console.log(step.id, step.tool?.kind);
});

plan.plan?.overlays?.forEach((overlay) => {
  console.log('Planner overlay', overlay.bbox);
});

if (plan.flow?.execution?.overlays) {
  for (const overlay of plan.flow.execution.overlays) {
    console.log('Screenshot path', overlay.screenshot_path);
  }
}

const artifacts = await client.getTaskArtifacts(plan.plan?.plan?.task_id!);
```

Schema reference：`docs/reference/PLAN_SCHEMA.md`（plan/flow 字段）与 `docs/ui/console_ia.md`（UI 消费方式）。

## Streaming

`openTaskStream(taskId)` returns an active `WebSocket` connected to the console. In
Node.js you need to pass a `WebSocketClass` implementation (for example from the
`ws` package) when instantiating `SoulBrowserClient`.

```ts
import { SoulBrowserClient } from 'soulbrowser-sdk';
import WebSocket from 'ws';

const client = new SoulBrowserClient({
  baseUrl: 'http://127.0.0.1:8801',
  WebSocketClass: WebSocket,
});

const socket = client.openTaskStream(taskId, { viaGateway: process.env.USE_GATEWAY === '1' });
socket.on('message', (event) => {
  const payload = JSON.parse(event.toString());
  console.log(payload.event, payload);
});

socket.on('message', (event) => {
  const payload = JSON.parse(event.toString());
  if (payload.event === 'overlay' && payload.overlay) {
    console.log('Live overlay', payload.overlay.source, payload.overlay.data);
  }
});
```

See `examples/sdk/typescript/live-companion.ts` for a runnable script that wires a
prompt execution + overlay stream using Node.js/`ws`.

### Server-Sent Events with resume support

`streamTaskEvents(taskId, options)` attaches to the SSE endpoint (`/api/tasks/:id/events`).
The helper automatically reconnects, tracks the latest `Last-Event-ID`, and exposes
connection state so dashboards can recover after network blips.

```ts
const stream = client.streamTaskEvents(taskId, { cursor: 0 });

const unsubscribe = stream.onEvent((event) => {
  console.log('task event', event.event);
});

stream.onConnectionChange((connected) => {
  console.log('sse connected?', connected, 'last id', stream.lastId);
});

stream.onError((err) => {
  console.error('stream error', err);
});

// Later
unsubscribe();
stream.close();
```

Provide `lastEventId` to resume from a stored checkpoint, or adjust
`retryDelayMs`/`maxRetryDelayMs` for noisy tunnels.

## Development

The SDK currently uses plain `tsc` for builds. To regenerate `dist/` run:

```
npm install
npm run build
```

The generated files target ES2020 modules. Adjust `tsconfig.json` if you need
CommonJS output.

> Building plugins/extensions? Pair this SDK with `docs/plugins/developer_guide.md`
> for manifest, review, and gateway authentication tips.
