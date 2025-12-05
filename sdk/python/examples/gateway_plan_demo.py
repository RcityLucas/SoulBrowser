"""Execute a simple AgentPlan via the Gateway and stream task events."""

from __future__ import annotations

import argparse
import os
import sys
import time
from dataclasses import asdict, dataclass, field
from typing import Any, Dict, List, Optional

import websocket  # type: ignore
from soulbrowser_sdk import SoulBrowserClient, SoulBrowserGatewayClient


def build_demo_plan(task_id: str) -> Dict[str, Any]:
    return {
        "task_id": task_id,
        "title": "Gateway Demo: Example Domain",
        "description": "Navigate to example.com and wait for the title to load.",
        "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "meta": {
            "rationale": ["Visit example.com and confirm the title"],
            "risk_assessment": [],
        },
        "steps": [
            {
                "id": "navigate",
                "title": "Navigate to example.com",
                "detail": "Open the target URL",
                "tool": {
                    "kind": {"Navigate": {"url": "https://example.com/"}},
                    "wait": "Idle",
                },
                "validations": [],
                "requires_approval": False,
                "metadata": {},
            },
            {
                "id": "wait-title",
                "title": "Wait for title",
                "detail": "Ensure the title matches Example Domain",
                "tool": {
                    "kind": {"Wait": {"condition": {"TitleMatches": "Example Domain"}}},
                    "wait": "DomReady",
                },
                "validations": [],
                "requires_approval": False,
                "metadata": {},
            },
        ],
    }


def to_ws_url(base_url: str, stream_path: str) -> str:
    gateway_url = base_url.rstrip("/")
    if stream_path.startswith("http"):
        return stream_path
    if gateway_url.startswith("https"):
        return gateway_url.replace("https", "wss", 1) + stream_path
    return gateway_url.replace("http", "ws", 1) + stream_path


def stream_task_events(ws_url: str) -> None:
    def _on_message(_: websocket.WebSocketApp, message: str) -> None:
        event = SoulBrowserClient.parse_task_stream_event(message)
        event_type = event.get("event")
        print("[stream]", event_type, event)
        if event_type == "status" and event.get("status", {}).get("status") in {"success", "failed"}:
            print("[stream] received final status, closing")
            sock.close()

    def _on_error(_: websocket.WebSocketApp, err: Exception) -> None:
        print("[stream.error]", err, file=sys.stderr)

    def _on_close(_: websocket.WebSocketApp, *__: object) -> None:
        print("[stream] closed")

    sock = websocket.WebSocketApp(ws_url, on_message=_on_message, on_error=_on_error, on_close=_on_close)
    sock.run_forever()


def main() -> None:
    parser = argparse.ArgumentParser(description="Gateway plan demo")
    parser.add_argument("--gateway", default=os.environ.get("SOULBROWSER_GATEWAY_URL", "http://127.0.0.1:8710"))
    parser.add_argument("--tenant", default=os.environ.get("SOULBROWSER_GATEWAY_TENANT", "demo-tenant"))
    parser.add_argument("--token", default=os.environ.get("SOULBROWSER_GATEWAY_TOKEN"))
    args = parser.parse_args()

    client = SoulBrowserGatewayClient(base_url=args.gateway, tenant_id=args.tenant, api_key=args.token)
    plan = build_demo_plan(task_id=f"gateway-demo-{int(time.time())}")

    payload = {
        "plan": plan,
        "prompt": "Capture Example Domain",
        "constraints": ["keep it simple"],
    }
    print("[gateway] POST /v1/tasks/run")
    response = client.run_plan(payload)
    if not response.get("success"):
        print("Gateway returned failure:", response)
        sys.exit(1)

    task_id = response["task_id"]
    ws_url = to_ws_url(args.gateway, response["stream_path"])
    print(f"[gateway] task_id={task_id}")
    print(f"[stream] connecting to {ws_url}")
    stream_task_events(ws_url)


if __name__ == "main":
    main()
