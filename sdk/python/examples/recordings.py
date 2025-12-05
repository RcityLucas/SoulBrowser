#!/usr/bin/env python3
"""List recorded sessions via the SoulBrowser Python SDK."""

import os
import sys
from typing import Optional

from soulbrowser_sdk.client import SoulBrowserClient

BASE_URL = os.environ.get('SOULBROWSER_API_BASE', 'http://127.0.0.1:8801')

def list_recordings(limit: Optional[int], state: Optional[str]) -> None:
    with SoulBrowserClient(base_url=BASE_URL) as client:
        resp = client.list_recordings(limit=limit, state=state)
        recordings = resp.get('recordings', [])
        if not recordings:
            print('No recordings found.')
            return
        for item in recordings:
            print(
                f"{item['id']} | state={item['state']} | has_plan={item['has_agent_plan']} | "
                f"updated={item['updated_at']}"
            )

def show_recording(session_id: str) -> None:
    with SoulBrowserClient(base_url=BASE_URL) as client:
        resp = client.get_recording(session_id)
        if not resp.get('success'):
            print('Recording not found or API returned failure.')
            return
        record = resp['recording']
        meta = record.get('metadata', {})
        print(f"Session: {record['id']}")
        print(f"State  : {record['state']}")
        print(f"Created: {record['created_at']} | Updated: {record['updated_at']}")
        print('Name   :', meta.get('name'))
        print('URL    :', meta.get('url'))
        if meta.get('agent_plan'):
            print('Plan   : embedded (use CLI/web to inspect JSON).')
        else:
            print('Plan   : not attached.')

if __name__ == '__main__':
    if len(sys.argv) > 1:
        show_recording(sys.argv[1])
    else:
        limit = int(os.environ.get('RECORDINGS_LIMIT', '10'))
        state = os.environ.get('RECORDINGS_STATE')
        list_recordings(limit, state)
