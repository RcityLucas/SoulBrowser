#!/usr/bin/env node
import WebSocket from 'ws';
import { SoulBrowserClient } from '../../sdk/typescript/dist/index.js';

const baseUrl = process.env.SOULBROWSER_API_BASE ?? 'http://127.0.0.1:8801';
const prompt = process.argv.slice(2).join(' ') || '打开 https://example.com 并截图';

async function run() {
  const client = new SoulBrowserClient({ baseUrl, WebSocketClass: WebSocket });
  const chat = await client.chat({
    prompt,
    execute: true,
    capture_context: true,
  });

  console.log('[chat] success:', chat.success);
  const plan = chat.plan?.plan;
  if (plan) {
    console.log('[plan] steps:', plan.steps?.length ?? 0);
  }

  const taskId = plan?.task_id;
  if (taskId) {
    console.log('[stream] watching task', taskId);
    const socket = client.openTaskStream(taskId);
    socket.on('message', (event) => {
      try {
        const payload = JSON.parse(event.toString());
        if (payload.event === 'overlay') {
          console.log('[overlay]', payload.overlay.source, payload.overlay.data?.bbox);
        }
      } catch (err) {
        console.error('[stream] failed to parse event', err);
      }
    });
    socket.on('close', () => process.exit(0));
  } else {
    console.log('No task id returned; nothing to stream.');
  }
}

run().catch((err) => {
  console.error(err);
  process.exit(1);
});
