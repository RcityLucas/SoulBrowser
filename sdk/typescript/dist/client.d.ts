import type { ChatRequest, ChatResponse, CreateTaskRequest, CreateTaskResponse, ExecuteTaskRequest, ExecuteTaskResponse, PerceiveRequest, PerceiveResponse, TaskArtifactsResponse, TaskDetailResponse, TaskLogEntry, TaskObservationsResponse, TaskStatusSnapshot, TaskSummary, TaskAnnotationsResponse, CreateTaskAnnotationRequest, TaskAnnotation, RecordingsListResponse, RecordingDetailResponse } from './types.js';
export interface ClientOptions {
    baseUrl?: string;
    fetchFn?: typeof fetch;
    WebSocketClass?: typeof WebSocket;
}
export interface TaskStreamOptions {
    viaGateway?: boolean;
    customPath?: string;
}
export declare class SoulBrowserClient {
    private baseUrl;
    private fetchFn;
    private WebSocketCtor?;
    constructor(options?: ClientOptions);
    setBaseUrl(url: string): void;
    getBaseUrl(): string;
    chat(payload: ChatRequest): Promise<ChatResponse>;
    perceive(payload: PerceiveRequest): Promise<PerceiveResponse>;
    createTask(payload: CreateTaskRequest): Promise<CreateTaskResponse>;
    listTasks(limit?: number): Promise<TaskSummary[]>;
    getTask(taskId: string, limit?: number): Promise<TaskDetailResponse>;
    getTaskStatus(taskId: string): Promise<TaskStatusSnapshot>;
    getTaskLogs(taskId: string, since?: string): Promise<TaskLogEntry[]>;
    getTaskObservations(taskId: string, limit?: number): Promise<TaskObservationsResponse>;
    listRecordings(limit?: number, state?: string): Promise<RecordingsListResponse>;
    getRecording(sessionId: string): Promise<RecordingDetailResponse>;
    getTaskArtifacts(taskId: string): Promise<TaskArtifactsResponse>;
    getTaskAnnotations(taskId: string): Promise<TaskAnnotationsResponse>;
    createTaskAnnotation(taskId: string, payload: CreateTaskAnnotationRequest): Promise<{
        success: boolean;
        annotation: TaskAnnotation;
    }>;
    executeTask(taskId: string, payload?: ExecuteTaskRequest): Promise<ExecuteTaskResponse>;
    cancelTask(taskId: string, reason?: string): Promise<{
        success: boolean;
    }>;
    openTaskStream(taskId: string, options?: TaskStreamOptions): WebSocket;
    private get;
    private post;
    private handleResponse;
}
export type { ChatRequest, ChatResponse, ContextSnapshot, CreateTaskRequest, CreateTaskResponse, ExecuteTaskRequest, ExecuteTaskResponse, PerceiveRequest, PerceiveResponse, TaskArtifactsResponse, TaskDetailResponse, TaskLogsResponse, TaskLogEntry, TaskObservationsResponse, RecordingDetailResponse, RecordingsListResponse, TaskStatusSnapshot, TaskStreamEvent, TaskSummary, TaskAnnotation, TaskAnnotationsResponse, CreateTaskAnnotationRequest, } from './types.js';
