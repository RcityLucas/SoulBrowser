#!/usr/bin/env python3
"""Fetch observation history for a task using the Python SDK."""

import os
import sys

from soulbrowser_sdk.client import SoulBrowserClient

BASE_URL = os.environ.get('SOULBROWSER_API_BASE', 'http://127.0.0.1:8801')


def resolve_task_id(client: SoulBrowserClient) -> str:
    if len(sys.argv) > 1:
        return sys.argv[1]
    tasks = list(client.list_tasks(limit=5))
    if not tasks:
        raise RuntimeError('No tasks available; pass a task id as the first argument.')
    for task in tasks:
        status = (task.get('last_status') or '').lower()
        if status == 'running':
            return task['task_id']
    return tasks[0]['task_id']


def main() -> None:
    limit = int(sys.argv[2]) if len(sys.argv) > 2 else 20
    with SoulBrowserClient(base_url=BASE_URL) as client:
        task_id = resolve_task_id(client)
        print(f'[observations] task={task_id} limit={limit}')
        response = client.get_task_observations(task_id, limit=limit)
        observations = response.get('observations', [])
        if not observations:
            print('No observations found.')
            return
        for idx, item in enumerate(observations, 1):
            obs_type = item.get('observation_type') or item.get('content_type') or 'unknown'
            recorded = item.get('recorded_at') or 'n/a'
            dispatch = item.get('dispatch_label') or '-'
            step = item.get('step_id') or '-'
            print(f"#{idx} [{obs_type}] step={step} dispatch={dispatch} recorded={recorded}")
            if item.get('bbox') is not None:
                print('    bbox:', item['bbox'])
            if item.get('screenshot_path'):
                print('    screenshot:', item['screenshot_path'])


if __name__ == '__main__':
    try:
        main()
    except Exception as exc:  # pragma: no cover - example script
        print(f'Error: {exc}', file=sys.stderr)
        sys.exit(1)
