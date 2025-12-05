from __future__ import annotations

import json
from typing import Any, Dict, Generator, Iterable, Optional, Type, Union

import httpx

from .types import (
    ChatRequest,
    ChatResponse,
    ContextSnapshot,
    CreateTaskAnnotationRequest,
    CreateTaskRequest,
    CreateTaskResponse,
    ExecuteTaskRequest,
    ExecuteTaskResponse,
    PerceiveRequest,
    PerceiveResponse,
    TaskAnnotation,
    TaskAnnotationsResponse,
    TaskArtifactsResponse,
    TaskDetailResponse,
    TaskLogEntry,
    TaskLogsResponse,
    TaskObservationsResponse,
    TaskStatusSnapshot,
    TaskStreamEvent,
    TaskSummary,
)

try:  # pragma: no cover - optional dependency
    import websocket  # type: ignore
except Exception:  # pragma: no cover
    websocket = None  # type: ignore


TaskStreamIterator = Generator[TaskStreamEvent, None, None]


class SoulBrowserClient:
    """Synchronous client for the SoulBrowser testing console APIs."""

    def __init__(
        self,
        base_url: str = "http://127.0.0.1:8801",
        *,
        timeout: float = 60.0,
        http: Optional[httpx.Client] = None,
        websocket_class: Optional[Type[Any]] = None,
    ) -> None:
        self._base_url = base_url.rstrip('/')
        self._owns_http = http is None
        self._http = http or httpx.Client(base_url=self._base_url, timeout=timeout)
        self._websocket_class = websocket_class or getattr(websocket, 'WebSocket', None)

    def close(self) -> None:
        if self._owns_http:
            self._http.close()

    # ---------------------------- REST helpers ----------------------------

    def chat(self, payload: ChatRequest) -> ChatResponse:
        return self._post('/api/chat', payload)

    def perceive(self, payload: PerceiveRequest) -> PerceiveResponse:
        return self._post('/api/perceive', payload)

    def create_task(self, payload: CreateTaskRequest) -> CreateTaskResponse:
        return self._post('/api/tasks', payload)

    def list_tasks(self, limit: Optional[int] = None) -> Iterable[TaskSummary]:
        params = {'limit': limit} if limit is not None else None
        data = self._get({'url': '/api/tasks', 'params': params})
        return data.get('tasks', [])

    def get_task(self, task_id: str, limit: Optional[int] = None) -> TaskDetailResponse:
        params = {'limit': limit} if limit is not None else None
        return self._get({'url': f'/api/tasks/{task_id}', 'params': params})

    def get_task_status(self, task_id: str) -> TaskStatusSnapshot:
        data = self._get({'url': f'/api/tasks/{task_id}/status'})
        return data['status']

    def get_task_logs(self, task_id: str, since: Optional[str] = None) -> Iterable[TaskLogEntry]:
        params = {'since': since} if since else None
        data: TaskLogsResponse = self._get({'url': f'/api/tasks/{task_id}/logs', 'params': params})
        return data['logs']

    def get_task_observations(
        self, task_id: str, limit: Optional[int] = None
    ) -> TaskObservationsResponse:
        params = {'limit': limit} if limit is not None else None
        return self._get({'url': f'/api/tasks/{task_id}/observations', 'params': params})

    def get_task_artifacts(self, task_id: str) -> TaskArtifactsResponse:
        return self._get({'url': f'/api/tasks/{task_id}/artifacts'})

    def get_task_annotations(self, task_id: str) -> TaskAnnotationsResponse:
        return self._get({'url': f'/api/tasks/{task_id}/annotations'})

    def list_recordings(
        self, limit: Optional[int] = None, state: Optional[str] = None
    ) -> RecordingsListResponse:
        params = {}
        if limit is not None:
            params['limit'] = limit
        if state:
            params['state'] = state
        return self._get({'url': '/api/recordings', 'params': params or None})

    def get_recording(self, session_id: str) -> RecordingDetailResponse:
        return self._get({'url': f'/api/recordings/{session_id}'})

    def create_task_annotation(
        self, task_id: str, payload: CreateTaskAnnotationRequest
    ) -> Dict[str, Any]:
        return self._post(f'/api/tasks/{task_id}/annotations', payload)

    def execute_task(
        self, task_id: str, payload: Optional[ExecuteTaskRequest] = None
    ) -> ExecuteTaskResponse:
        return self._post(f'/api/tasks/{task_id}/execute', payload or {})

    def cancel_task(self, task_id: str, reason: Optional[str] = None) -> Dict[str, Any]:
        return self._post(f'/api/tasks/{task_id}/cancel', {'reason': reason})

    # ---------------------------- streaming ----------------------------

    def open_task_stream(
        self,
        task_id: str,
        *,
        via_gateway: bool = False,
        path: Optional[str] = None,
    ):
        if not self._websocket_class:
            raise RuntimeError(
                'No WebSocket implementation available. Install `websocket-client` '
                'or pass `websocket_class` to the client.'
            )
        target_path = path or (
            f'/v1/tasks/{task_id}/stream' if via_gateway else f'/api/tasks/{task_id}/stream'
        )
        url = self._build_ws_url(target_path)
        ws = self._websocket_class()
        ws.connect(url)
        return ws

    def iter_task_stream(self, ws) -> TaskStreamIterator:
        while True:
            message = ws.recv()
            if message is None:
                break
            yield json.loads(message)

    # ---------------------------- internals ----------------------------

    def _get(self, request: Dict[str, Any]) -> Any:
        resp = self._http.get(request['url'], params=request.get('params'))
        resp.raise_for_status()
        return resp.json()

    def _post(self, url: str, payload: Union[Dict[str, Any], ChatRequest]) -> Any:
        resp = self._http.post(url, json=payload)
        resp.raise_for_status()
        return resp.json()

    def _build_ws_url(self, path: str) -> str:
        base = httpx.URL(self._base_url)
        scheme = 'wss' if base.scheme == 'https' else 'ws'
        return f"{scheme}://{base.host}:{base.port or (443 if scheme == 'wss' else 80)}{path}"

    # context manager helpers -------------------------------------------------

    def __enter__(self) -> 'SoulBrowserClient':
        return self

    def __exit__(self, *exc_info: Any) -> None:
        self.close()
