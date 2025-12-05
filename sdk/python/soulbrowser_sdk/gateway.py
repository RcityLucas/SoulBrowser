from __future__ import annotations

from typing import Any, Dict, Optional

import httpx

from .types import GatewayPlanRunResponse, GatewayToolRunRequest, GatewayToolRunResponse


class SoulBrowserGatewayClient:
    """Minimal client for the L7 Gateway (`/v1/tools/run`)."""

    def __init__(
        self,
        *,
        tenant_id: str,
        base_url: str = "http://127.0.0.1:8710",
        timeout: float = 30.0,
        http: Optional[httpx.Client] = None,
        api_key: Optional[str] = None,
        tenant_token: Optional[str] = None,
        headers: Optional[Dict[str, str]] = None,
    ) -> None:
        if not tenant_id:
            raise ValueError("tenant_id is required")
        self._base_url = base_url.rstrip('/')
        self._tenant_id = tenant_id
        self._owns_http = http is None
        self._http = http or httpx.Client(base_url=self._base_url, timeout=timeout)
        self._api_key = api_key
        self._tenant_token = tenant_token
        self._static_headers = headers or {}

    def close(self) -> None:
        if self._owns_http:
            self._http.close()

    def set_tenant(self, tenant_id: str) -> None:
        if not tenant_id:
            raise ValueError("tenant_id must not be empty")
        self._tenant_id = tenant_id

    def configure_auth(self, *, api_key: Optional[str] = None, tenant_token: Optional[str] = None) -> None:
        if api_key is not None:
            self._api_key = api_key
        if tenant_token is not None:
            self._tenant_token = tenant_token

    def run_tool(
        self,
        payload: GatewayToolRunRequest,
        *,
        tenant_id: Optional[str] = None,
        headers: Optional[Dict[str, str]] = None,
    ) -> GatewayToolRunResponse:
        if not payload.get('tool'):
            raise ValueError("'tool' is required in the payload")
        request_headers = self._build_headers(tenant_id=tenant_id, overrides=headers)
        response = self._http.post('/v1/tools/run', json=payload, headers=request_headers)
        response.raise_for_status()
        return response.json()

    def run_plan(
        self,
        payload: Dict[str, Any],
        *,
        tenant_id: Optional[str] = None,
        headers: Optional[Dict[str, str]] = None,
    ) -> GatewayPlanRunResponse:
        if not payload.get('plan'):
            raise ValueError("'plan' is required in the payload")
        request_headers = self._build_headers(tenant_id=tenant_id, overrides=headers)
        response = self._http.post('/v1/tasks/run', json=payload, headers=request_headers)
        response.raise_for_status()
        data = response.json()
        return GatewayPlanRunResponse(**data)

    def _build_headers(
        self,
        *,
        tenant_id: Optional[str] = None,
        overrides: Optional[Dict[str, str]] = None,
    ) -> Dict[str, str]:
        headers: Dict[str, str] = {
            'content-type': 'application/json',
            'x-tenant-id': tenant_id or self._tenant_id,
            **self._static_headers,
        }
        if overrides:
            headers.update(overrides)
        lowered = {key.lower() for key in headers}
        if self._api_key and 'authorization' not in lowered:
            headers['Authorization'] = f'Bearer {self._api_key}'
        if self._tenant_token and 'x-tenant-token' not in headers:
            headers['x-tenant-token'] = self._tenant_token
        return headers

    def __enter__(self) -> 'SoulBrowserGatewayClient':
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()
