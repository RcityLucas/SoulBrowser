#!/usr/bin/env ts-node
import { randomUUID } from 'crypto';
import WebSocket from 'ws';

interface GatewayRunResponse {
  success: boolean;
  task_id: string;
  stream_path: string;
  error?: string;
}

interface TaskStreamEvent {
  event: string;
  [key: string]: any;
}

const parseArgs = () => {
  const args = process.argv.slice(2);
  const options: Record<string, string> = {};
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg.startsWith('--')) {
      const key = arg.slice(2);
      const value = args[i + 1];
      if (value && !value.startsWith('--')) {
        options[key] = value;
        i += 1;
      } else {
        options[key] = 'true';
      }
    }
  }
  return options;
};

const options = parseArgs();
const gatewayBase = options.gateway ?? process.env.GATEWAY_BASE ?? 'http://127.0.0.1:8710';
const tenantId = options.tenant ?? process.env.GATEWAY_TENANT ?? 'demo-tenant';

const demoPlan = (taskId: string) => ({
  task_id: taskId,
  title: 'Gateway Demo: Capture Example Domain',
  description: 'Navigate to https://example.com and wait for the title to render.',
  created_at: new Date().toISOString(),
  meta: { rationale: ['Visit example.com and confirm the title is visible.'], risk_assessment: [] },
  steps: [
    {
      id: 'step-1',
      title: 'Navigate to example.com',
      detail: 'Open the target URL using the navigate tool.',
      tool: {
        kind: { Navigate: { url: 'https://example.com/' } },
        wait: 'Idle',
      },
      validations: [],
      requires_approval: false,
      metadata: {},
    },
    {
      id: 'step-2',
      title: 'Wait for title',
      detail: 'Ensure the browser title contains "Example Domain".',
      tool: {
        kind: { Wait: { condition: { TitleMatches: 'Example Domain' } } },
        wait: 'DomReady',
      },
      validations: [],
      requires_approval: false,
      metadata: {},
    },
  ],
});

const buildHeaders = () => {
  const headers: Record<string, string> = {
    'content-type': 'application/json',
    'x-tenant-id': tenantId,
  };
  if (process.env.GATEWAY_TOKEN) {
    headers.authorization = `Bearer ${process.env.GATEWAY_TOKEN}`;
  }
  return headers;
};

const toWebSocketUrl = (base: string, path: string) => {
  const url = new URL(path, base);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  return url.toString();
};

const run = async () => {
  const taskId = options.task ?? randomUUID();
  const payload = {
    plan: demoPlan(taskId),
    prompt: 'Capture the Example Domain landing page and log when the title appears.',
    constraints: ['Prefer simple navigation', 'Do not click additional links'],
  };

  console.log('[gateway] POST /v1/tasks/run');
  const response = await fetch(new URL('/v1/tasks/run', gatewayBase), {
    method: 'POST',
    headers: buildHeaders(),
    body: JSON.stringify(payload),
  });

  if (!response.ok) {
    const errorText = await response.text().catch(() => '');
    throw new Error(`Gateway run failed: ${response.status} ${response.statusText} ${errorText}`);
  }

  const body = (await response.json()) as GatewayRunResponse;
  if (!body.success) {
    throw new Error(body.error || 'Gateway returned failure status');
  }

  console.log('[gateway] task_id =', body.task_id);
  const streamUrl = toWebSocketUrl(gatewayBase, body.stream_path);
  console.log('[stream] connecting to', streamUrl);

  const socket = new WebSocket(streamUrl, {
    headers: buildHeaders(),
  });

  socket.on('open', () => {
    console.log('[stream] connected');
  });

  socket.on('message', (data: WebSocket.RawData) => {
    try {
      const event: TaskStreamEvent = JSON.parse(data.toString());
      if (event.event === 'status') {
        console.log('[status]', event.status?.status, 'current step:', event.status?.current_step_title);
        if (event.status?.status === 'success' || event.status?.status === 'failed') {
          console.log('[stream] final status received, closing connection');
          socket.close();
        }
      } else if (event.event === 'overlay') {
        console.log('[overlay]', event.overlay?.source, event.overlay?.data?.title ?? event.overlay?.data?.dispatch_label);
      } else if (event.event === 'error') {
        console.warn('[stream-error]', event.message);
      }
    } catch (err) {
      console.error('[stream] failed to parse event', err);
    }
  });

  socket.on('close', () => {
    console.log('[stream] closed');
    process.exit(0);
  });

  socket.on('error', (err) => {
    console.error('[stream] error', err);
    process.exit(1);
  });
};

run().catch((err) => {
  console.error(err);
  process.exit(1);
});
