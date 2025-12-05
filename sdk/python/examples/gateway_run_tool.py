"""Sample script demonstrating the SoulBrowserGatewayClient."""

from __future__ import annotations

import os

from soulbrowser_sdk import SoulBrowserGatewayClient


def main() -> None:
    tenant_id = os.environ.get("SOULBROWSER_GATEWAY_TENANT", "demo-tenant")
    token = os.environ.get("SOULBROWSER_GATEWAY_TOKEN")

    with SoulBrowserGatewayClient(tenant_id=tenant_id, api_key=token) as gateway:
        response = gateway.run_tool(
            {
                "tool": "navigate",
                "params": {"url": "https://example.com"},
                "timeout_ms": 15_000,
            }
        )
        print("status:", response.get("status"))
        print("trace:", response.get("trace_id"))


if __name__ == "__main__":
    main()
