export interface ContextSnapshot {
    success?: boolean;
    perception?: Record<string, unknown>;
    screenshot_base64?: string;
    stdout?: string;
    stderr?: string;
    error?: unknown;
}
export interface ChatRequest {
    prompt: string;
    current_url?: string;
    constraints?: string[];
    execute?: boolean;
    planner?: string;
    llm_provider?: string;
    llm_model?: string;
    llm_api_base?: string;
    llm_temperature?: number;
    llm_api_key?: string;
    llm_max_output_tokens?: number;
    capture_context?: boolean;
    context_timeout_secs?: number;
    context_screenshot?: boolean;
}
export interface ChatResponse {
    success: boolean;
    plan?: unknown;
    flow?: unknown;
    artifacts?: unknown;
    context?: ContextSnapshot;
    stdout?: string;
    stderr?: string;
}
export interface CreateTaskRequest {
    prompt: string;
    current_url?: string;
    constraints?: string[];
    planner?: string;
    llm_provider?: string;
    llm_model?: string;
    llm_api_base?: string;
    llm_temperature?: number;
    llm_api_key?: string;
    llm_max_output_tokens?: number;
    execute?: boolean;
    max_replans?: number;
    max_retries?: number;
    capture_context?: boolean;
    context_timeout_secs?: number;
    context_screenshot?: boolean;
}
export interface CreateTaskResponse {
    success: boolean;
    task_id?: string;
    stdout: string[];
    stderr: string[];
    plan?: unknown;
    flow?: unknown;
    artifacts?: unknown;
    context?: ContextSnapshot;
}
export interface TaskSummary {
    task_id: string;
    total_dispatches: number;
    success_count: number;
    failure_count: number;
    last_status?: string | null;
    last_tool?: string | null;
    last_error?: string | null;
    last_recorded_at?: string | null;
    prompt?: string | null;
    created_at?: string | null;
    source?: string | null;
    planner?: string | null;
    llm_provider?: string | null;
    llm_model?: string | null;
}
export interface TaskStatusSnapshot {
    task_id: string;
    title: string;
    status: string;
    total_steps: number;
    current_step?: number | null;
    current_step_title?: string | null;
    started_at?: string | null;
    finished_at?: string | null;
    last_error?: string | null;
    last_updated_at: string;
    plan_overlays?: unknown;
    recent_evidence?: unknown[];
    observation_history?: unknown[];
    context_snapshot?: ContextSnapshot;
}
export interface TaskLogEntry {
    timestamp: string;
    level: 'info' | 'warn' | 'error';
    message: string;
}
export interface TaskDispatchSummary {
    action_id: string;
    status: string;
    tool: string;
    attempts: number;
    wait_ms: number;
    run_ms: number;
    pending: number;
    slots_available: number;
    route: Record<string, unknown>;
    error?: string | null;
    output?: Record<string, unknown> | null;
    recorded_at: string;
}
export interface PersistedPlanRecord {
    task_id: string;
    prompt: string;
    created_at: string;
    source: string;
    plan: Record<string, unknown>;
    flow: Record<string, unknown>;
    explanations: string[];
    summary: string[];
    constraints: string[];
    current_url?: string | null;
    planner: string;
    llm_provider?: string | null;
    llm_model?: string | null;
    context_snapshot?: ContextSnapshot | null;
}
export interface TaskDetailResponse {
    success: boolean;
    summary: TaskSummary;
    dispatches: TaskDispatchSummary[];
    plan?: PersistedPlanRecord | null;
}
export interface TaskArtifactsResponse {
    success: boolean;
    task_id: string;
    items: Record<string, unknown>[];
    summary: Record<string, unknown>;
}
export interface TaskLogsResponse {
    success: boolean;
    logs: TaskLogEntry[];
}
export interface TaskObservationsResponse {
    success: boolean;
    task_id: string;
    observations: Record<string, unknown>[];
}
export interface RecordingSummary {
    id: string;
    state: string;
    created_at: string;
    updated_at: string;
    name?: string;
    url?: string | null;
    has_agent_plan: boolean;
}
export interface RecordingsListResponse {
    success: boolean;
    recordings: RecordingSummary[];
}
export interface RecordingDetailResponse {
    success: boolean;
    recording: {
        id: string;
        state: string;
        created_at: string;
        updated_at: string;
        metadata: Record<string, unknown>;
    };
}
export interface TaskAnnotation {
    id: string;
    step_id?: string | null;
    dispatch_label?: string | null;
    note: string;
    bbox?: unknown;
    author?: string | null;
    severity?: string | null;
    created_at: string;
}
export interface CreateTaskAnnotationRequest {
    step_id?: string;
    dispatch_label?: string;
    note: string;
    bbox?: unknown;
    author?: string;
    severity?: string;
}
export interface TaskAnnotationsResponse {
    success: boolean;
    annotations: TaskAnnotation[];
}
export type OverlaySource = 'plan' | 'execution';
export interface ObservationEventPayload {
    observation_type?: string;
    task_id?: string;
    step_id?: string | null;
    dispatch_label?: string | null;
    dispatch_index?: number | null;
    screenshot_path?: string | null;
    bbox?: unknown;
    content_type?: string | null;
    recorded_at?: string | null;
    artifact: Record<string, unknown>;
}
export interface OverlayEventPayload {
    task_id: string;
    source: OverlaySource;
    data: Record<string, unknown>;
    recorded_at: string;
}
export type TaskStreamEvent = {
    event: 'status';
    status: TaskStatusSnapshot;
} | {
    event: 'log';
    log: TaskLogEntry;
} | {
    event: 'context';
    context: ContextSnapshot;
} | ({
    event: 'observation';
} & ObservationEventPayload) | {
    event: 'overlay';
    overlay: OverlayEventPayload;
} | {
    event: 'annotation';
    annotation: TaskAnnotation;
} | {
    event: 'error';
    message: string;
};
export interface PerceiveRequest {
    url: string;
    mode?: 'all' | 'structural' | 'visual' | 'semantic';
    screenshot?: boolean;
    insights?: boolean;
    structural?: boolean;
    visual?: boolean;
    semantic?: boolean;
    timeout?: number;
}
export interface PerceiveResponse {
    success: boolean;
    stdout?: string;
    stderr?: string;
    error?: string;
    perception?: Record<string, unknown>;
    screenshot_base64?: string;
}
export interface ExecuteTaskRequest {
    max_retries?: number;
}
export interface ExecuteTaskResponse {
    success: boolean;
    stdout: string[];
    stderr: string[];
    report: Record<string, unknown>;
    artifacts?: unknown;
}
export interface GatewayToolRunRequest {
    tool: string;
    params?: Record<string, unknown>;
    routing?: Record<string, unknown>;
    options?: Record<string, unknown>;
    timeout_ms?: number;
    idempotency_key?: string;
    trace_id?: string;
}
export interface GatewayToolRunResponse {
    status: string;
    data?: Record<string, unknown>;
    trace_id?: string;
    action_id?: string;
}
