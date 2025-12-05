#!/usr/bin/env node
import WebSocket from 'ws';
import { SoulBrowserClient } from '../../sdk/typescript/dist/index.js';

const baseUrl = process.env.SOULBROWSER_API_BASE ?? 'http://127.0.0.1:8801';
const taskIdArg = process.argv[2];
const limit = Number(process.argv[3] ?? '20');

async function resolveTaskId(client) {
  if (taskIdArg) {
    return taskIdArg;
  }
  const tasks = await client.listTasks(5);
  const active = tasks.find((task) => task.last_status?.toLowerCase() === 'running');
  if (active) {
    return active.task_id;
  }
  if (tasks.length > 0) {
    return tasks[0].task_id;
  }
  throw new Error('No tasks available. Provide a task id as the first argument.');
}

async function run() {
  const client = new SoulBrowserClient({ baseUrl, WebSocketClass: WebSocket });
  const taskId = await resolveTaskId(client);
  console.log(`[observations] querying task ${taskId} (limit=${limit})`);
  const response = await client.getTaskObservations(taskId, limit);
  if (!response.success) {
    console.error('API returned failure payload:', response);
    process.exit(1);
  }
  if (!response.observations.length) {
    console.log('No observations found.');
    return;
  }
  response.observations.forEach((item, index) => {
    const type = item.observation_type || item.content_type || 'unknown';
    const recorded = item.recorded_at || 'n/a';
    const desc = item.dispatch_label || item.label || item.step_id || 'entry';
    console.log(`#${index + 1} [${type}] step=${item.step_id ?? '-'} dispatch=${item.dispatch_label ?? '-'} recorded=${recorded}`);
    if (item.bbox) {
      console.log('    bbox:', JSON.stringify(item.bbox));
    }
    if (item.screenshot_path) {
      console.log('    screenshot:', item.screenshot_path);
    }
  });
}

run().catch((err) => {
  console.error(err);
  process.exit(1);
});
