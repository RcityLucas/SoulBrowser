"""Minimal live companion demo.

Requires `websocket-client` to stream events:
    pip install soulbrowser-sdk[ws]

Usage:
    python live_companion.py --prompt "打开 example.com" \
        --base-url http://127.0.0.1:8801 \
        [--gateway --gateway-url http://127.0.0.1:8710]
"""

from __future__ import annotations

import argparse
import json
import time
from typing import Any

from soulbrowser_sdk import SoulBrowserClient


def pretty_event(event: dict[str, Any]) -> str:
    name = event.get("event")
    if name == "overlay":
        overlay = event.get("overlay", {})
        return f"overlay[{overlay.get('source')}]: {overlay.get('data', {}).get('title')}"
    if name == "annotation":
        annotation = event.get("annotation", {})
        return f"annotation: {annotation.get('note')}"
    if name == "status":
        status = event.get("status", {})
        return f"status: {status.get('status').upper()}"
    return name or "event"


def main() -> None:
    parser = argparse.ArgumentParser(description="Live task companion demo")
    parser.add_argument('--prompt', required=True, help='Prompt to send to /api/chat')
    parser.add_argument('--base-url', default='http://127.0.0.1:8801')
    parser.add_argument('--gateway', action='store_true', help='Stream via the Gateway /v1 endpoint')
    parser.add_argument('--gateway-url', default='http://127.0.0.1:8710')
    args = parser.parse_args()

    client = SoulBrowserClient(base_url=args.gateway_url if args.gateway else args.base_url)
    chat = client.chat({
        'prompt': args.prompt,
        'execute': True,
        'capture_context': True,
    })
    plan = chat.get('plan', {}).get('plan', {})
    task_id = plan.get('task_id')
    if not task_id:
        raise SystemExit('Planner did not return a task id')
    print(f"Task created: {task_id}")

    socket = client.open_task_stream(task_id, via_gateway=args.gateway)
    print('Streaming events... ctrl+c to exit')
    try:
        for event in client.iter_task_stream(socket):
            print(pretty_event(event))
            if event.get('event') == 'status':
                status = event['status'].get('status')
                if status in {'success', 'failed'}:
                    break
    except KeyboardInterrupt:
        print('Interrupted by user')
    finally:
        socket.close()
        # give server a moment to flush
        time.sleep(0.2)

    print('\nFinal plan summary:')
    print(json.dumps(chat.get('plan', {}), ensure_ascii=False, indent=2))

if __name__ == '__main__':
    main()
