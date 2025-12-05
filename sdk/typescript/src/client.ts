import type {
  ChatRequest,
  ChatResponse,
  CreateTaskRequest,
  CreateTaskResponse,
  ExecuteTaskRequest,
  ExecuteTaskResponse,
  PerceiveRequest,
  PerceiveResponse,
  TaskArtifactsResponse,
  TaskDetailResponse,
  TaskLogsResponse,
  TaskLogEntry,
  TaskObservationsResponse,
  TaskStatusSnapshot,
  TaskStreamEvent,
  TaskSummary,
  TaskAnnotationsResponse,
  CreateTaskAnnotationRequest,
  TaskAnnotation,
  RecordingsListResponse,
  RecordingDetailResponse,
} from './types.js';

export interface ClientOptions {
  baseUrl?: string;
  fetchFn?: typeof fetch;
  WebSocketClass?: typeof WebSocket;
}

export interface TaskStreamOptions {
  viaGateway?: boolean;
  customPath?: string;
}

export class SoulBrowserClient {
  private baseUrl: string;
  private fetchFn: typeof fetch;
  private WebSocketCtor?: typeof WebSocket;

  constructor(options?: ClientOptions) {
    this.baseUrl = options?.baseUrl ?? 'http://127.0.0.1:8801';
    this.fetchFn = options?.fetchFn ?? globalThis.fetch;
    this.WebSocketCtor = options?.WebSocketClass ?? (globalThis as any).WebSocket;

    if (!this.fetchFn) {
      throw new Error('No fetch implementation available. Pass `fetchFn` in the constructor.');
    }
  }

  setBaseUrl(url: string) {
    this.baseUrl = url;
  }

  getBaseUrl() {
    return this.baseUrl;
  }

  async chat(payload: ChatRequest): Promise<ChatResponse> {
    return this.post<ChatResponse>('/api/chat', payload);
  }

  async perceive(payload: PerceiveRequest): Promise<PerceiveResponse> {
    return this.post<PerceiveResponse>('/api/perceive', payload);
  }

  async createTask(payload: CreateTaskRequest): Promise<CreateTaskResponse> {
    return this.post<CreateTaskResponse>('/api/tasks', payload);
  }

  async listTasks(limit?: number): Promise<TaskSummary[]> {
    const params = limit ? `?limit=${limit}` : '';
    const data = await this.get<{ success: boolean; tasks: TaskSummary[] }>(`/api/tasks${params}`);
    return data.tasks;
  }

  async getTask(taskId: string, limit?: number): Promise<TaskDetailResponse> {
    const params = limit ? `?limit=${limit}` : '';
    return this.get(`/api/tasks/${taskId}${params}`);
  }

  async getTaskStatus(taskId: string): Promise<TaskStatusSnapshot> {
    const data = await this.get<{ success: boolean; status: TaskStatusSnapshot }>(
      `/api/tasks/${taskId}/status`
    );
    return data.status;
  }

  async getTaskLogs(taskId: string, since?: string): Promise<TaskLogEntry[]> {
    const params = since ? `?since=${encodeURIComponent(since)}` : '';
    const data = await this.get<TaskLogsResponse>(`/api/tasks/${taskId}/logs${params}`);
    return data.logs;
  }

  async getTaskObservations(
    taskId: string,
    limit?: number
  ): Promise<TaskObservationsResponse> {
    const params = limit ? `?limit=${limit}` : '';
    return this.get(`/api/tasks/${taskId}/observations${params}`);
  }

  async listRecordings(limit?: number, state?: string): Promise<RecordingsListResponse> {
    const params = new URLSearchParams();
    if (typeof limit === 'number') params.set('limit', String(limit));
    if (state) params.set('state', state);
    const query = params.toString();
    const suffix = query.length ? `?${query}` : '';
    return this.get(`/api/recordings${suffix}`);
  }

  async getRecording(sessionId: string): Promise<RecordingDetailResponse> {
    return this.get(`/api/recordings/${sessionId}`);
  }

  async getTaskArtifacts(taskId: string): Promise<TaskArtifactsResponse> {
    return this.get(`/api/tasks/${taskId}/artifacts`);
  }

  async getTaskAnnotations(taskId: string): Promise<TaskAnnotationsResponse> {
    return this.get(`/api/tasks/${taskId}/annotations`);
  }

  async createTaskAnnotation(
    taskId: string,
    payload: CreateTaskAnnotationRequest,
  ): Promise<{ success: boolean; annotation: TaskAnnotation }> {
    return this.post(`/api/tasks/${taskId}/annotations`, payload);
  }

  async executeTask(
    taskId: string,
    payload?: ExecuteTaskRequest
  ): Promise<ExecuteTaskResponse> {
    return this.post(`/api/tasks/${taskId}/execute`, payload ?? {});
  }

  async cancelTask(taskId: string, reason?: string): Promise<{ success: boolean }> {
    return this.post(`/api/tasks/${taskId}/cancel`, { reason });
  }

  openTaskStream(taskId: string, options?: TaskStreamOptions): WebSocket {
    if (!this.WebSocketCtor) {
      throw new Error('No WebSocket implementation is available. Pass `WebSocketClass` in options.');
    }
    const base = new URL(this.baseUrl);
    base.protocol = base.protocol === 'https:' ? 'wss:' : 'ws:';
    const defaultPath = options?.viaGateway
      ? `/v1/tasks/${taskId}/stream`
      : `/api/tasks/${taskId}/stream`;
    const customPath = options?.customPath;
    base.pathname = customPath ? customPath.replace(':task_id', taskId) : defaultPath;
    base.search = '';
    base.hash = '';
    return new this.WebSocketCtor(base.toString());
  }

  private async get<T>(path: string): Promise<T> {
    const response = await this.fetchFn(new URL(path, this.baseUrl), {
      method: 'GET',
      headers: {
        'Content-Type': 'application/json',
      },
    });
    return this.handleResponse<T>(response);
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const response = await this.fetchFn(new URL(path, this.baseUrl), {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(body ?? {}),
    });
    return this.handleResponse<T>(response);
  }

  private async handleResponse<T>(response: Response): Promise<T> {
    if (!response.ok) {
      const text = await response.text().catch(() => '');
      throw new Error(`Request failed: ${response.status} ${response.statusText} ${text}`);
    }
    return (await response.json()) as T;
  }
}

export type {
  ChatRequest,
  ChatResponse,
  ContextSnapshot,
  CreateTaskRequest,
  CreateTaskResponse,
  ExecuteTaskRequest,
  ExecuteTaskResponse,
  PerceiveRequest,
  PerceiveResponse,
  TaskArtifactsResponse,
  TaskDetailResponse,
  TaskLogsResponse,
  TaskLogEntry,
  TaskObservationsResponse,
  RecordingDetailResponse,
  RecordingsListResponse,
  TaskStatusSnapshot,
  TaskStreamEvent,
  TaskSummary,
  TaskAnnotation,
  TaskAnnotationsResponse,
  CreateTaskAnnotationRequest,
} from './types.js';
