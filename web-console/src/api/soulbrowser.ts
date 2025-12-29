/**
 * SoulBrowser specific API calls
 */

import axios, { AxiosInstance } from 'axios';
import type {
  CreateSessionRequest,
  SessionRecord,
  SessionShareContext,
  SessionSnapshot,
} from '@/types';

export interface PerceiveRequest {
  url: string;
  mode: 'all' | 'structural' | 'visual' | 'semantic' | 'custom';
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
  perception?: {
    structural?: {
      dom_node_count?: number;
      form_count?: number;
      interactive_count?: number;
      text_node_count?: number;
    };
    visual?: {
      dominant_colors?: string[];
      viewport_width?: number;
      viewport_height?: number;
    };
    semantic?: {
      content_type?: string;
      main_heading?: string;
      language?: string;
    };
    insights?: Array<{
      type: string;
      message: string;
      severity?: string;
    }>;
  };
  screenshot_base64?: string;
}

export interface PerceptionMetrics {
  total_runs: number;
  shared_hits: number;
  shared_misses: number;
  shared_failures: number;
  ephemeral_runs: number;
  failed_runs: number;
  avg_duration_ms: number;
}

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
  llm_max_output_tokens?: number;
  capture_context?: boolean;
  context_timeout_secs?: number;
  context_screenshot?: boolean;
  session_id?: string;
  profile_id?: string;
  profile_label?: string;
}

export interface ChatResponse {
  success: boolean;
  plan?: any;
  flow?: any;
  artifacts?: any;
  context?: ContextSnapshot;
  session_id?: string;
  stdout?: string;
  stderr?: string;
}

export interface TaskSummary {
  task_id: string;
  prompt: string;
  created_at: string;
  source: string;
  path: string;
  planner?: string | null;
  llm_provider?: string | null;
  llm_model?: string | null;
  session_id?: string | null;
}

export interface PersistedPlanRecord {
  version: number;
  task_id: string;
  prompt: string;
  created_at: string;
  source: string;
  plan: any;
  flow: Record<string, unknown>;
  explanations: string[];
  summary: string[];
  constraints: string[];
  current_url?: string | null;
  session_id?: string | null;
  planner: string;
  llm_provider?: string | null;
  llm_model?: string | null;
  context_snapshot?: ContextSnapshot | null;
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
  plan_overlays?: any;
  recent_evidence?: any[];
  observation_history?: any[];
  context_snapshot?: ContextSnapshot;
  annotations?: TaskAnnotation[];
  agent_history?: AgentHistoryEntry[];
  user_results?: TaskUserResult[];
  missing_user_result?: boolean;
  watchdog_events?: WatchdogEvent[];
  judge_verdict?: TaskJudgeVerdict | null;
  self_heal_events?: SelfHealEvent[];
  alerts?: TaskAlert[];
}

export interface TaskLogEntry {
  timestamp: string;
  level: 'info' | 'warn' | 'error';
  message: string;
}

export interface AgentHistoryEntry {
  timestamp: string;
  step_index: number;
  step_id: string;
  title: string;
  status: 'success' | 'failed';
  attempts: number;
  message?: string | null;
  observation_summary?: string | null;
  obstruction?: string | null;
  structured_summary?: string | null;
  tool_kind?: string | null;
  wait_ms?: number | null;
  run_ms?: number | null;
}

export interface WatchdogEvent {
  id: string;
  kind: string;
  severity: string;
  note: string;
  step_id?: string | null;
  dispatch_label?: string | null;
  recorded_at: string;
}

export interface SelfHealEvent {
  timestamp: number;
  strategy_id: string;
  action: string;
  note?: string | null;
}

export interface TaskAlert {
  timestamp: string;
  severity: string;
  message: string;
  kind?: string | null;
}

export interface JudgeVerdict {
  passed: boolean;
  reason?: string | null;
}

export interface TaskJudgeVerdict {
  verdict: JudgeVerdict;
  recorded_at: string;
}

interface SessionListResponse {
  success: boolean;
  sessions: SessionRecord[];
}

interface SessionCreateResponse {
  success: boolean;
  session: SessionRecord;
}

interface SessionDetailResponseApi {
  success: boolean;
  snapshot: SessionSnapshot;
}

interface SessionShareResponse {
  success: boolean;
  share: SessionShareContext;
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

export interface TaskDetailResponse {
  success: boolean;
  task: any;
}

export interface TaskExecutionsResponse {
  success: boolean;
  executions: any[];
}

export interface ArtifactListItem {
  attempt: number;
  step_index: number;
  step_id: string;
  dispatch_label: string;
  dispatch_index: number;
  artifact_index: number;
  action_id: string;
  label: string;
  content_type: string;
  byte_len: number;
  filename?: string | null;
  path?: string | null;
  data_base64?: string;
}

export interface ArtifactSummary {
  count: number;
  total_bytes: number;
  by_content_type: Record<string, number>;
}

export interface TaskArtifactsResponse {
  success: boolean;
  task_id: string;
  items: ArtifactListItem[];
  summary: ArtifactSummary;
}

export interface TaskListResponse {
  success: boolean;
  tasks: TaskSummary[];
}

export interface TaskStatusResponse {
  success: boolean;
  status: TaskStatusSnapshot;
}

export interface TaskLogsResponse {
  success: boolean;
  logs: TaskLogEntry[];
}

export interface TaskObservationsResponse {
  success: boolean;
  task_id: string;
  observations: Record<string, any>[];
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
  kind?: string | null;
  created_at: string;
}

export interface TaskUserResult {
  step_id: string;
  step_title: string;
  kind: string;
  schema?: string | null;
  content?: unknown;
  artifact_path?: string | null;
}

export interface CreateTaskAnnotationRequest {
  step_id?: string;
  dispatch_label?: string;
  note: string;
  bbox?: unknown;
  author?: string;
  severity?: string;
  kind?: string;
}

export interface TaskAnnotationsResponse {
  success: boolean;
  annotations: TaskAnnotation[];
}

export type OverlaySource = 'plan' | 'execution';

export interface OverlayEventPayload {
  task_id: string;
  source: OverlaySource;
  data: Record<string, unknown>;
  recorded_at: string;
}

export type TaskStreamEvent =
  | { event: 'status'; status: TaskStatusSnapshot }
  | { event: 'log'; log: TaskLogEntry }
  | { event: 'context'; context: ContextSnapshot }
  | {
      event: 'observation';
      observation_type: string;
      task_id: string;
      step_id?: string | null;
      dispatch_label?: string | null;
      dispatch_index?: number | null;
      screenshot_path?: string | null;
      bbox?: unknown;
      content_type?: string | null;
      recorded_at?: string | null;
      artifact: Record<string, unknown>;
    }
  | { event: 'overlay'; overlay: OverlayEventPayload }
  | { event: 'annotation'; annotation: TaskAnnotation }
  | { event: 'agent_history'; entry: AgentHistoryEntry }
  | { event: 'watchdog'; watchdog: WatchdogEvent }
  | { event: 'judge'; verdict: TaskJudgeVerdict }
  | { event: 'self_heal'; self_heal: SelfHealEvent }
  | { event: 'alert'; alert: TaskAlert }
  | { event: 'error'; message: string };

export interface TaskExecuteResponse {
  success: boolean;
  stdout: string[];
  stderr: string[];
  report: Record<string, unknown>;
  artifacts?: any;
}

export interface TaskExecuteRequest {
  max_retries?: number;
}

export interface TaskCancelResponse {
  success: boolean;
  cancelled: number;
  message: string;
}

export interface CreateTaskRequest {
  prompt?: string;
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
  template_id?: string;
  template_ref?: string;
}

export interface CreateTaskResponse {
  success: boolean;
  task_id?: string;
  stdout: string[];
  stderr: string[];
  plan?: any;
  flow?: any;
  artifacts?: any;
  context?: ContextSnapshot | null;
}

export interface MemoryRecord {
  id: string;
  namespace: string;
  key: string;
  tags: string[];
  note?: string | null;
  metadata?: Record<string, unknown> | null;
  created_at: string;
  use_count: number;
  success_count: number;
  last_used_at?: string | null;
}

export interface MemoryStatsSnapshot {
  total_queries: number;
  hit_queries: number;
  miss_queries: number;
  hit_rate: number;
  stored_records: number;
  deleted_records: number;
  current_records: number;
  template_uses: number;
  template_successes: number;
  template_success_rate: number;
}

export interface MemoryStatsWithTrends extends MemoryStatsSnapshot {
  avg_query_rate?: number;
  exporter_status?: string;
}

export interface MemoryListParams {
  namespace?: string;
  tag?: string;
  limit?: number;
}

export interface MemoryListResponse {
  success: boolean;
  records: MemoryRecord[];
}

export interface CreateMemoryRecordRequest {
  namespace: string;
  key: string;
  tags?: string[];
  note?: string;
  metadata?: Record<string, unknown>;
}

export interface CreateMemoryRecordResponse {
  success: boolean;
  record?: MemoryRecord;
  error?: string;
}

export interface DeleteMemoryRecordResponse {
  success: boolean;
  error?: string;
}

export interface UpdateMemoryRecordRequest {
  tags?: string[];
  note?: string | null;
  metadata?: unknown | null;
}

type TaggedSelfHealAction =
  | ({ type: 'auto_retry' } & { extra_attempts?: number })
  | ({ type: 'annotate' } & { severity?: string; note?: string })
  | ({ type: 'human_approval' } & { severity?: string });

type LegacySelfHealAction =
  | { auto_retry: { extra_attempts?: number } }
  | { annotate: { severity?: string; note?: string } }
  | { human_approval: { severity?: string } };

type RawSelfHealAction = TaggedSelfHealAction | LegacySelfHealAction;

interface RawSelfHealStrategy {
  id: string;
  description: string;
  enabled: boolean;
  tags: string[];
  telemetry_label?: string | null;
  action: RawSelfHealAction;
}

export type SelfHealAction =
  | { kind: 'auto_retry'; extra_attempts: number }
  | { kind: 'annotate'; severity?: string; note?: string }
  | { kind: 'human_approval'; severity?: string };

export interface SelfHealStrategy {
  id: string;
  description: string;
  enabled: boolean;
  tags: string[];
  telemetry_label?: string | null;
  action: SelfHealAction;
}

export interface PluginRegistryRecord {
  id: string;
  status?: string | null;
  owner?: string | null;
  last_reviewed_at?: string | null;
  description?: string | null;
  scopes?: string[] | null;
  helpers?: RegistryHelper[];
}

export interface RegistryHelper {
  id: string;
  pattern: string;
  description?: string | null;
  prompt?: string | null;
  auto_insert?: boolean;
  blockers?: string[];
  step?: RegistryHelperStep;
  steps?: RegistryHelperStep[];
  conditions?: HelperConditions;
}

export interface RegistryHelperStep {
  title: string;
  detail?: string | null;
  wait?: string | null;
  timeout_ms?: number | null;
  tool: RegistryHelperTool;
}

export type RegistryHelperTool =
  | { type: 'click_css'; selector: string }
  | { type: 'click_text'; text: string; exact?: boolean }
  | { type: 'custom'; name: string; payload?: Record<string, unknown> };

export interface HelperConditions {
  url_includes?: string[];
  url_excludes?: string[];
}

export interface HelperScaffoldParams {
  id: string;
  pattern: string;
  description?: string;
  prompt?: string;
  auto_insert?: boolean;
  blockers?: string[];
  step_title?: string;
  step_detail?: string;
  step_wait?: string;
  selector?: string;
}

export interface PluginRegistryStats {
  total_registry_entries: number;
  active_plugins: number;
  pending_review: number;
  last_reviewed_at?: string | null;
  registry_path?: string | null;
}

export interface GatewayRunResponse {
  success: boolean;
  task_id: string;
  stream_path: string;
}

export interface SelfHealEvent {
  timestamp: number;
  strategy_id: string;
  action: string;
  note?: string | null;
}

const envBase = import.meta.env.VITE_API_BASE_URL?.trim();
const runtimeOrigin =
  typeof window !== 'undefined' && window.location?.origin ? window.location.origin : undefined;
const DEFAULT_BASE_URL =
  envBase && envBase.length > 0
    ? envBase
    : runtimeOrigin && runtimeOrigin.length > 0
    ? runtimeOrigin
    : 'http://127.0.0.1:8804';
const DEFAULT_SERVE_TOKEN = 'soulbrowser-dev-token';

export function resolveServeToken(): string | null {
  if (typeof window !== 'undefined' && typeof localStorage !== 'undefined') {
    const stored = localStorage.getItem('auth_token') || localStorage.getItem('serve_token');
    if (stored && stored.trim()) {
      return stored.trim();
    }
  }
  const envToken = import.meta.env.VITE_SERVE_TOKEN;
  if (envToken && envToken.trim()) {
    return envToken.trim();
  }
  return DEFAULT_SERVE_TOKEN;
}

const normalizeSelfHealAction = (action: RawSelfHealAction): SelfHealAction => {
  if ('type' in action) {
    switch (action.type) {
      case 'auto_retry':
        return { kind: 'auto_retry', extra_attempts: action.extra_attempts ?? 0 };
      case 'annotate':
        return { kind: 'annotate', severity: action.severity, note: action.note };
      case 'human_approval':
        return { kind: 'human_approval', severity: action.severity };
      default:
        return { kind: 'annotate' };
    }
  }

  if ('auto_retry' in action) {
    return {
      kind: 'auto_retry',
      extra_attempts: action.auto_retry?.extra_attempts ?? 0,
    };
  }
  if ('annotate' in action) {
    return {
      kind: 'annotate',
      severity: action.annotate?.severity,
      note: action.annotate?.note,
    };
  }
  if ('human_approval' in action) {
    return {
      kind: 'human_approval',
      severity: action.human_approval?.severity,
    };
  }

  return { kind: 'annotate' };
};

const normalizeSelfHealStrategy = (strategy: RawSelfHealStrategy): SelfHealStrategy => ({
  id: strategy.id,
  description: strategy.description,
  enabled: strategy.enabled,
  tags: strategy.tags ?? [],
  telemetry_label: strategy.telemetry_label ?? undefined,
  action: normalizeSelfHealAction(strategy.action),
});

class SoulBrowserAPI {
  private client: AxiosInstance;
  private baseURL: string;

  constructor(baseURL: string = DEFAULT_BASE_URL) {
    this.baseURL = baseURL;
    this.client = axios.create({
      baseURL: baseURL || undefined,
      timeout: 60000, // 60 seconds for browser operations
      headers: {
        'Content-Type': 'application/json',
      },
    });

    this.client.interceptors.request.use(
      (config) => {
        const token = resolveServeToken();
        if (token) {
          config.headers = config.headers ?? {};
          config.headers['x-soulbrowser-token'] = token;
          config.headers.Authorization = `Bearer ${token}`;
        }
        return config;
      },
      (error) => Promise.reject(error)
    );
  }

  setBaseUrl(baseURL: string) {
    this.baseURL = baseURL;
    this.client.defaults.baseURL = baseURL || undefined;
  }

  getBaseUrl() {
    return this.baseURL;
  }

  /**
   * Execute multi-modal page perception
   */
  async perceive(request: PerceiveRequest): Promise<PerceiveResponse> {
    const response = await this.client.post<PerceiveResponse>('/api/perceive', request);
    return response.data;
  }

  async getPerceptionMetrics(): Promise<PerceptionMetrics> {
    const response = await this.client.get<{ success: boolean; metrics: PerceptionMetrics }>(
      '/api/perceive/metrics'
    );
    return response.data.metrics;
  }

  /**
   * Generate task plan using L8 agent
   */
  async chat(request: ChatRequest): Promise<ChatResponse> {
    const response = await this.client.post<ChatResponse>('/api/chat', request);
    return response.data;
  }

  async listTasks(limit?: number): Promise<TaskSummary[]> {
    const response = await this.client.get<TaskListResponse>('/api/tasks', {
      params: limit ? { limit } : undefined,
    });
    return response.data.tasks;
  }

  async getTask(taskId: string, limit?: number): Promise<TaskDetailResponse> {
    const response = await this.client.get<TaskDetailResponse>(`/api/tasks/${taskId}`, {
      params: limit ? { limit } : undefined,
    });
    return response.data;
  }

  async getTaskStatus(taskId: string): Promise<TaskStatusSnapshot> {
    const response = await this.client.get<TaskStatusResponse>(`/api/tasks/${taskId}/status`);
    return response.data.status;
  }

  async getTaskExecutions(taskId: string): Promise<any[]> {
    const response = await this.client.get<TaskExecutionsResponse>(
      `/api/tasks/${taskId}/executions`
    );
    return response.data.executions;
  }

  async getTaskLogs(taskId: string, since?: string): Promise<TaskLogEntry[]> {
    const response = await this.client.get<TaskLogsResponse>(`/api/tasks/${taskId}/logs`, {
      params: since ? { since } : undefined,
    });
    return response.data.logs;
  }

  async getTaskObservations(taskId: string, limit?: number): Promise<TaskObservationsResponse> {
    const response = await this.client.get<TaskObservationsResponse>(
      `/api/tasks/${taskId}/observations`,
      { params: limit ? { limit } : undefined }
    );
    return response.data;
  }

  async listRecordings(limit?: number, state?: string): Promise<RecordingsListResponse> {
    const response = await this.client.get<RecordingsListResponse>(`/api/recordings`, {
      params: {
        ...(limit ? { limit } : {}),
        ...(state ? { state } : {}),
      },
    });
    return response.data;
  }

  async getRecording(sessionId: string): Promise<RecordingDetailResponse> {
    const response = await this.client.get<RecordingDetailResponse>(`/api/recordings/${sessionId}`);
    return response.data;
  }

  async getTaskArtifacts(taskId: string): Promise<TaskArtifactsResponse> {
    const response = await this.client.get<TaskArtifactsResponse>(
      `/api/tasks/${taskId}/artifacts`
    );
    return response.data;
  }

  async getTaskAnnotations(taskId: string): Promise<TaskAnnotation[]> {
    const response = await this.client.get<TaskAnnotationsResponse>(
      `/api/tasks/${taskId}/annotations`
    );
    return response.data.annotations;
  }

  async createTaskAnnotation(
    taskId: string,
    payload: CreateTaskAnnotationRequest,
  ): Promise<TaskAnnotation> {
    const response = await this.client.post<{ success: boolean; annotation: TaskAnnotation }>(
      `/api/tasks/${taskId}/annotations`,
      payload,
    );
    return response.data.annotation;
  }

  async downloadTaskArtifact(taskId: string, artifactName: string): Promise<Blob> {
    const response = await this.client.get<Blob>(
      `/api/tasks/${taskId}/artifacts/${encodeURIComponent(artifactName)}`,
      { responseType: 'blob' }
    );
    return response.data;
  }

  openTaskStream(taskId: string): WebSocket {
    const base = this.baseURL || window.location.origin;
    const url = new URL(base);
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
    url.pathname = `/api/tasks/${taskId}/stream`;
    url.search = '';
    url.hash = '';
    return new WebSocket(url.toString());
  }

  async executeTask(
    taskId: string,
    payload?: TaskExecuteRequest,
  ): Promise<TaskExecuteResponse> {
    const response = await this.client.post<TaskExecuteResponse>(
      `/api/tasks/${taskId}/execute`,
      payload ?? {}
    );
    return response.data;
  }

  async cancelTask(taskId: string, reason?: string): Promise<TaskCancelResponse> {
    const response = await this.client.post<TaskCancelResponse>(
      `/api/tasks/${taskId}/cancel`,
      { reason }
    );
    return response.data;
  }

  async createTask(payload: CreateTaskRequest): Promise<CreateTaskResponse> {
    const response = await this.client.post<CreateTaskResponse>('/api/tasks', payload);
    return response.data;
  }

  async listMemoryRecords(params?: MemoryListParams): Promise<MemoryRecord[]> {
    const response = await this.client.get<MemoryListResponse>('/api/memory', {
      params,
    });
    return response.data.records;
  }

  async createMemoryRecord(payload: CreateMemoryRecordRequest): Promise<MemoryRecord> {
    const response = await this.client.post<CreateMemoryRecordResponse>('/api/memory', payload);
    if (!response.data.success || !response.data.record) {
      throw new Error(response.data.error || 'Failed to store memory record');
    }
    return response.data.record;
  }

  async deleteMemoryRecord(id: string): Promise<void> {
    const response = await this.client.delete<DeleteMemoryRecordResponse>(`/api/memory/${id}`);
    if (!response.data.success) {
      throw new Error(response.data.error || 'Failed to delete memory record');
    }
  }

  async updateMemoryRecord(
    id: string,
    payload: UpdateMemoryRecordRequest
  ): Promise<MemoryRecord> {
    const response = await this.client.put<{
      success: boolean;
      record?: MemoryRecord;
      error?: string;
    }>(`/api/memory/${id}`, payload);
    if (!response.data.success || !response.data.record) {
      throw new Error(response.data.error || 'Failed to update memory record');
    }
    return response.data.record;
  }

  async getMemoryStats(): Promise<MemoryStatsSnapshot> {
    const response = await this.client.get<{ success: boolean; stats: MemoryStatsSnapshot }>(
      '/api/memory/stats'
    );
    if (!response.data.success) {
      throw new Error('Failed to fetch memory stats');
    }
    return response.data.stats;
  }

  async listSelfHealStrategies(): Promise<SelfHealStrategy[]> {
    const response = await this.client.get<{
      success: boolean;
      strategies: RawSelfHealStrategy[];
      error?: string;
    }>('/api/self-heal/strategies');
    if (!response.data.success) {
      throw new Error(response.data.error || 'Failed to load self-heal strategies');
    }
    return response.data.strategies.map(normalizeSelfHealStrategy);
  }

  async setSelfHealStrategyEnabled(strategyId: string, enabled: boolean): Promise<void> {
    const response = await this.client.post<{
      success: boolean;
      error?: string;
    }>(`/api/self-heal/strategies/${strategyId}`, { enabled });
    if (!response.data.success) {
      throw new Error(response.data.error || 'Failed to update strategy');
    }
  }

  async listPluginRegistry(): Promise<{
    stats: PluginRegistryStats;
    plugins: PluginRegistryRecord[];
  }> {
    const response = await this.client.get<{
      success: boolean;
      stats: PluginRegistryStats;
      plugins: PluginRegistryRecord[];
      error?: string;
    }>('/api/plugins/registry');
    if (!response.data.success) {
      throw new Error(response.data.error || 'Failed to load plugin registry');
    }
    return {
      stats: response.data.stats,
      plugins: response.data.plugins,
    };
  }

  async updatePluginStatus(
    pluginId: string,
    status: 'active' | 'pending' | 'disabled'
  ): Promise<PluginRegistryRecord> {
    const response = await this.client.post<{
      success: boolean;
      plugin?: PluginRegistryRecord;
      stats?: PluginRegistryStats;
      error?: string;
    }>(`/api/plugins/registry/${pluginId}`, { status });
    if (!response.data.success || !response.data.plugin) {
      throw new Error(response.data.error || 'Failed to update plugin status');
    }
    return response.data.plugin;
  }

  async listPluginHelpers(pluginId: string): Promise<RegistryHelper[]> {
    const response = await this.client.get<{
      success: boolean;
      helpers?: RegistryHelper[];
      error?: string;
    }>(`/api/plugins/registry/${pluginId}/helpers`);
    if (!response.data.success) {
      throw new Error(response.data.error || 'Failed to load helpers');
    }
    return response.data.helpers ?? [];
  }

  async scaffoldPluginHelper(
    pluginId: string,
    params: HelperScaffoldParams
  ): Promise<RegistryHelper> {
    const response = await this.client.get<{ helper?: RegistryHelper; error?: string }>(
      `/api/plugins/registry/${pluginId}/helpers/scaffold`,
      {
        params: {
          ...params,
          blockers: params.blockers && params.blockers.length > 0 ? params.blockers.join(',') : undefined,
        },
      }
    );
    if (!response.data.helper) {
      throw new Error(response.data.error || 'Failed to scaffold helper');
    }
    return response.data.helper;
  }

  async createPluginHelper(
    pluginId: string,
    helper: RegistryHelper
  ): Promise<RegistryHelper> {
    const response = await this.client.post<{
      success: boolean;
      helper?: RegistryHelper;
      error?: string;
    }>(`/api/plugins/registry/${pluginId}/helpers`, { helper });
    if (!response.data.success || !response.data.helper) {
      throw new Error(response.data.error || 'Failed to create helper');
    }
    return response.data.helper;
  }

  async updatePluginHelper(
    pluginId: string,
    helperId: string,
    helper: RegistryHelper
  ): Promise<RegistryHelper> {
    const response = await this.client.put<{
      success: boolean;
      helper?: RegistryHelper;
      error?: string;
    }>(`/api/plugins/registry/${pluginId}/helpers/${helperId}`, { helper });
    if (!response.data.success || !response.data.helper) {
      throw new Error(response.data.error || 'Failed to update helper');
    }
    return response.data.helper;
  }

  async deletePluginHelper(pluginId: string, helperId: string): Promise<void> {
    const response = await this.client.delete<{
      success: boolean;
      error?: string;
    }>(`/api/plugins/registry/${pluginId}/helpers/${helperId}`);
    if (!response.data.success) {
      throw new Error(response.data.error || 'Failed to delete helper');
    }
  }

  async runGatewayPlan(record: PersistedPlanRecord): Promise<GatewayRunResponse> {
    const planPayload = { ...(record.plan as Record<string, unknown>) };
    if ('overlays' in planPayload) {
      delete (planPayload as Record<string, unknown>).overlays;
    }
    const contextSnapshot = record.current_url
      ? {
          current_url: record.current_url,
        }
      : undefined;
    const response = await this.client.post<GatewayRunResponse & { success: boolean; error?: string }>(
      '/v1/tasks/run',
      {
        plan: planPayload,
        prompt: record.prompt,
        constraints: record.constraints ?? [],
        context: contextSnapshot,
      }
    );
    if (!response.data.success) {
      throw new Error(response.data.error || 'Gateway 执行失败');
    }
    return {
      success: response.data.success,
      task_id: response.data.task_id,
      stream_path: response.data.stream_path,
    };
  }

  async listSessions(): Promise<SessionRecord[]> {
    const response = await this.client.get<SessionListResponse>('/api/sessions');
    return response.data.sessions;
  }

  async createSession(payload?: CreateSessionRequest): Promise<SessionRecord> {
    const response = await this.client.post<SessionCreateResponse>(
      '/api/sessions',
      payload ?? {}
    );
    return response.data.session;
  }

  async getSessionSnapshot(sessionId: string): Promise<SessionSnapshot> {
    const response = await this.client.get<SessionDetailResponseApi>(
      `/api/sessions/${sessionId}`
    );
    return response.data.snapshot;
  }

  async issueSessionShare(sessionId: string): Promise<SessionShareContext> {
    const response = await this.client.post<SessionShareResponse>(
      `/api/sessions/${sessionId}/share`
    );
    return response.data.share;
  }

  async revokeSessionShare(sessionId: string): Promise<SessionShareContext> {
    const response = await this.client.delete<SessionShareResponse>(
      `/api/sessions/${sessionId}/share`
    );
    return response.data.share;
  }

  openSessionStream(sessionId: string, shareToken?: string): EventSource {
    const base = this.baseURL || window.location.origin;
    const url = new URL(base);
    url.pathname = `/api/sessions/${sessionId}/live`;
    url.search = '';
    if (shareToken) {
      url.searchParams.set('share', shareToken);
    }
    const token = resolveServeToken();
    if (token) {
      url.searchParams.set('token', token);
    }
    return new EventSource(url.toString());
  }

  /**
   * Health check
   */
  async health(): Promise<{ status: string }> {
    const response = await this.client.get('/health');
    return response.data;
  }
}

export const soulbrowserAPI = new SoulBrowserAPI();
export default soulbrowserAPI;
