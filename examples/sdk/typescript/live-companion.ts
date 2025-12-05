import WebSocket from 'ws';
import { SoulBrowserClient } from 'soulbrowser-sdk';

async function main() {
  const baseUrl = process.env.SOULBROWSER_BASE_URL ?? 'http://127.0.0.1:8801';
  const viaGateway = process.env.SOULBROWSER_USE_GATEWAY === '1';
  const prompt = process.env.SOULBROWSER_PROMPT ?? '打开 example.com 并截图';

  const client = new SoulBrowserClient({
    baseUrl: viaGateway ? process.env.SOULBROWSER_GATEWAY_URL ?? 'http://127.0.0.1:8710' : baseUrl,
    WebSocketClass: WebSocket,
  });

  const chat = await client.chat({ prompt, execute: true, capture_context: true });
  const taskId = chat.plan?.plan?.task_id;
  if (!taskId) {
    throw new Error('Planner did not return a task id');
  }
  console.log('Task created:', taskId);

  const socket = client.openTaskStream(taskId, { viaGateway });
  socket.on('message', (raw) => {
    const payload = JSON.parse(raw.toString());
    if (payload.event === 'overlay') {
      console.log('[overlay]', payload.overlay?.source, payload.overlay?.data?.title);
    } else if (payload.event === 'annotation') {
      console.log('[annotation]', payload.annotation?.note);
    } else if (payload.event === 'status') {
      console.log('[status]', payload.status?.status);
      if (payload.status?.status === 'success' || payload.status?.status === 'failed') {
        socket.close();
      }
    }
  });

  socket.on('close', () => {
    console.log('stream closed');
  });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
