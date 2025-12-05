from __future__ import annotations

from typing import Any, Dict, List, Optional, TypedDict


class ContextSnapshot(TypedDict, total=False):
    success: bool
    perception: Dict[str, Any]
    screenshot_base64: str
    stdout: str
    stderr: str
    error: Any


class ChatRequest(TypedDict, total=False):
    prompt: str
    current_url: str
    constraints: List[str]
    execute: bool
    planner: str
    llm_provider: str
    llm_model: str
    llm_api_base: str
    llm_temperature: float
    llm_api_key: str
    llm_max_output_tokens: int
    capture_context: bool
    context_timeout_secs: int
    context_screenshot: bool


class ChatResponse(TypedDict, total=False):
    success: bool
    plan: Any
    flow: Any
    artifacts: Any
    context: ContextSnapshot
    stdout: str
    stderr: str


class CreateTaskRequest(TypedDict, total=False):
    prompt: str
    current_url: str
    constraints: List[str]
    planner: str
    llm_provider: str
    llm_model: str
    llm_api_base: str
    llm_temperature: float
    llm_api_key: str
    llm_max_output_tokens: int
    execute: bool
    max_replans: int
    max_retries: int
    capture_context: bool
    context_timeout_secs: int
    context_screenshot: bool


class CreateTaskResponse(TypedDict, total=False):
    success: bool
    task_id: str
    stdout: List[str]
    stderr: List[str]
    plan: Any
    flow: Any
    artifacts: Any
    context: ContextSnapshot


class TaskSummary(TypedDict, total=False):
    task_id: str
    total_dispatches: int
    success_count: int
    failure_count: int
    last_status: Optional[str]
    last_tool: Optional[str]
    last_error: Optional[str]
    last_recorded_at: Optional[str]
    prompt: Optional[str]
    created_at: Optional[str]
    source: Optional[str]
    planner: Optional[str]
    llm_provider: Optional[str]
    llm_model: Optional[str]


class TaskStatusSnapshot(TypedDict, total=False):
    task_id: str
    title: str
    status: str
    total_steps: int
    current_step: Optional[int]
    current_step_title: Optional[str]
    started_at: Optional[str]
    finished_at: Optional[str]
    last_error: Optional[str]
    last_updated_at: str
    plan_overlays: Any
    recent_evidence: List[Any]
    observation_history: List[Any]
    context_snapshot: Optional[ContextSnapshot]


class TaskLogEntry(TypedDict, total=False):
    timestamp: str
    level: str
    message: str


class TaskDispatchSummary(TypedDict, total=False):
    action_id: str
    status: str
    tool: str
    attempts: int
    wait_ms: int
    run_ms: int
    pending: int
    slots_available: int
    route: Dict[str, Any]
    error: Optional[str]
    output: Optional[Dict[str, Any]]
    recorded_at: str


class PersistedPlanRecord(TypedDict, total=False):
    task_id: str
    prompt: str
    created_at: str
    source: str
    plan: Dict[str, Any]
    flow: Dict[str, Any]
    explanations: List[str]
    summary: List[str]
    constraints: List[str]
    current_url: Optional[str]
    planner: str
    llm_provider: Optional[str]
    llm_model: Optional[str]
    context_snapshot: Optional[ContextSnapshot]


class TaskDetailResponse(TypedDict, total=False):
    success: bool
    summary: TaskSummary
    dispatches: List[TaskDispatchSummary]
    plan: Optional[PersistedPlanRecord]
    annotations: List[TaskAnnotation]


class TaskArtifactsResponse(TypedDict, total=False):
    success: bool
    task_id: str
    items: List[Dict[str, Any]]
    summary: Dict[str, Any]


class TaskLogsResponse(TypedDict, total=False):
    success: bool
    logs: List[TaskLogEntry]


class TaskObservationsResponse(TypedDict, total=False):
    success: bool
    task_id: str
    observations: List[Dict[str, Any]]


class RecordingSummary(TypedDict, total=False):
    id: str
    state: str
    created_at: str
    updated_at: str
    name: Optional[str]
    url: Optional[str]
    has_agent_plan: bool


class RecordingsListResponse(TypedDict, total=False):
    success: bool
    recordings: List[RecordingSummary]


class RecordingDetailResponse(TypedDict, total=False):
    success: bool
    recording: Dict[str, Any]


class TaskAnnotation(TypedDict, total=False):
    id: str
    step_id: Optional[str]
    dispatch_label: Optional[str]
    note: str
    bbox: Any
    author: Optional[str]
    severity: Optional[str]
    created_at: str


class GatewayToolRunRequest(TypedDict, total=False):
    tool: str
    params: Dict[str, Any]
    routing: Dict[str, Any]
    options: Dict[str, Any]
    timeout_ms: int
    idempotency_key: str
    trace_id: str


class GatewayToolRunResponse(TypedDict, total=False):
    status: str
    data: Dict[str, Any]
    trace_id: Optional[str]
    action_id: Optional[str]


class GatewayPlanRunResponse(TypedDict, total=False):
    success: bool
    task_id: str
    stream_path: str


class TaskAnnotationsResponse(TypedDict, total=False):
    success: bool
    annotations: List[TaskAnnotation]


class CreateTaskAnnotationRequest(TypedDict, total=False):
    step_id: Optional[str]
    dispatch_label: Optional[str]
    note: str
    bbox: Any
    author: Optional[str]
    severity: Optional[str]


class TaskStreamStatusEvent(TypedDict):
    event: str
    status: TaskStatusSnapshot


class TaskStreamLogEvent(TypedDict):
    event: str
    log: TaskLogEntry


class TaskStreamContextEvent(TypedDict):
    event: str
    context: ContextSnapshot


class TaskStreamObservationEvent(TypedDict):
    event: str
    observation_type: Optional[str]
    task_id: Optional[str]
    step_id: Optional[str]
    dispatch_label: Optional[str]
    dispatch_index: Optional[int]
    screenshot_path: Optional[str]
    bbox: Any
    content_type: Optional[str]
    recorded_at: Optional[str]
    artifact: Dict[str, Any]


class OverlayPayload(TypedDict, total=False):
    task_id: str
    source: str
    data: Dict[str, Any]
    recorded_at: str


class TaskStreamOverlayEvent(TypedDict):
    event: str
    overlay: OverlayPayload


class TaskStreamAnnotationEvent(TypedDict):
    event: str
    annotation: TaskAnnotation


class TaskStreamErrorEvent(TypedDict):
    event: str
    message: str


TaskStreamEvent = (
    TaskStreamStatusEvent
    | TaskStreamLogEvent
    | TaskStreamContextEvent
    | TaskStreamObservationEvent
    | TaskStreamOverlayEvent
    | TaskStreamAnnotationEvent
    | TaskStreamErrorEvent
)


class PerceiveRequest(TypedDict, total=False):
    url: str
    mode: str
    screenshot: bool
    insights: bool
    structural: bool
    visual: bool
    semantic: bool
    timeout: int


class PerceiveResponse(TypedDict, total=False):
    success: bool
    stdout: str
    stderr: str
    error: str
    perception: Dict[str, Any]
    screenshot_base64: str


class ExecuteTaskRequest(TypedDict, total=False):
    max_retries: int


class ExecuteTaskResponse(TypedDict, total=False):
    success: bool
    stdout: List[str]
    stderr: List[str]
    report: Dict[str, Any]
    artifacts: Any
