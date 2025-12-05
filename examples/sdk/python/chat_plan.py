#!/usr/bin/env python3
import argparse
import json
import os
import sys
import time

from soulbrowser_sdk import SoulBrowserClient


def main() -> None:
    parser = argparse.ArgumentParser(description="SoulBrowser Python SDK example")
    parser.add_argument('--base-url', default=os.environ.get('SOULBROWSER_API_BASE', 'http://127.0.0.1:8801'))
    parser.add_argument('--prompt', default='打开 https://example.com 并截图')
    args = parser.parse_args()

    client = SoulBrowserClient(base_url=args.base_url)
    try:
        chat = client.chat({
            'prompt': args.prompt,
            'execute': True,
            'capture_context': True,
        })
        plan = chat.get('plan', {}).get('plan', {})
        print('[chat] success:', chat.get('success'))
        print('[plan] steps:', len(plan.get('steps', [])))
        task_id = plan.get('task_id')
        if not task_id:
            print('No task id returned; exiting')
            return
        print('[stream] watching task', task_id)
        ws = client.open_task_stream(task_id)
        try:
            for event in client.iter_task_stream(ws):
                if event.get('event') == 'overlay':
                    overlay = event.get('overlay', {})
                    print('[overlay]', overlay.get('source'), overlay.get('data'))
                elif event.get('event') == 'status' and event.get('status', {}).get('status') in {'success', 'failed'}:
                    break
        finally:
            ws.close()
    finally:
        client.close()


if __name__ == '__main__':
    main()
