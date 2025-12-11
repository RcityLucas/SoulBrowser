# SoulBrowser Python SDK (preview)

Companion SDK for automating the SoulBrowser testing console APIs from Python scripts.
It wraps the `/api/chat`, `/api/tasks/*`, and `/api/perceive` endpoints exposed by
`soulbrowser serve`, plus a helper for the task WebSocket stream.

> **Status**: preview / non-published. Package metadata lives in `pyproject.toml`.

## Installation

```
pip install ./sdk/python          # local editable install
# or build wheel / publish via `python -m build`
```

The SDK uses [`httpx`](https://www.python-httpx.org/) under the hood. To receive live
stream events you can optionally install `websocket-client`:

```
pip install "soulbrowser-sdk[ws]"
```

## Examples

```python
from soulbrowser_sdk import SoulBrowserClient

client = SoulBrowserClient(base_url="http://127.0.0.1:8801")
chat = client.chat({
    "prompt": "open example.com and capture a screenshot",
    "execute": False,
    "capture_context": True,
    "current_url": "https://example.com",
})
print(chat["plan"]["plan"]["title"])

# Planner overlays
for overlay in chat.get("plan", {}).get("overlays", []):
    print("overlay bbox", overlay.get("bbox"))

execution = chat.get("flow", {}).get("execution", {})
for overlay in execution.get("overlays", []) or []:
    print("screenshot", overlay.get("screenshot_path"))

socket = client.open_task_stream(chat["plan"]["plan"]["task_id"], via_gateway=True)
for event in client.iter_task_stream(socket):
    print(event["event"], event)

# Overlay events mirror docs/reference/api_TASK_CENTER.md
    if event.get("event") == "overlay":
        overlay = event.get("overlay", {})
        print("overlay source", overlay.get("source"), "data", overlay.get("data"))

# Future overlay/annotation events follow docs/reference/PLAN_SCHEMA.md

- `examples/sdk/python/gateway_plan_demo.py` demonstrates the `/v1/tasks/run` endpoint
  and streams events via WebSocket.

Gateway (L7 Adapter) usage for REST tools / Browser Use alignments:

```python
from soulbrowser_sdk import SoulBrowserGatewayClient

gateway = SoulBrowserGatewayClient(
    tenant_id="demo-tenant",
    base_url="http://127.0.0.1:8710",
    api_key="your-token",
)

response = gateway.run_tool(
    {
        "tool": "navigate",
        "params": {"url": "https://example.com"},
        "timeout_ms": 15000,
    }
)

print("gateway status", response.get("status"), "trace", response.get("trace_id"))
```

## Development

```
cd sdk/python
python -m venv .venv
source .venv/bin/activate
pip install -e .[ws]
```

Run formatting / linting as needed before publishing.

> Plan to publish plugins or browser extensions? Combine this SDK with
> `docs/plugins/developer_guide.md` for manifest, testing, and review workflow tips.
