/**
 * HTTP API Client
 */

import axios, { AxiosInstance } from 'axios';
import { message } from 'antd';
import type { Task, CreateTaskRequest, TaskFilter, MetricsQuery, MetricsReport } from '@/types';
import { soulbrowserAPI, resolveServeToken } from '@/api/soulbrowser';

class ApiClient {
  private client: AxiosInstance;
  private baseUrl?: string;

  constructor() {
    this.baseUrl = deriveApiBaseUrl();
    this.client = axios.create({
      baseURL: this.baseUrl,
      timeout: 30000,
      headers: {
        'Content-Type': 'application/json',
      },
    });

    // Request interceptor
    this.client.interceptors.request.use(
      (config) => {
        const baseURL = this.baseUrl ?? deriveApiBaseUrl();
        if (baseURL) {
          config.baseURL = baseURL;
        }
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

    // Response interceptor
    this.client.interceptors.response.use(
      (response) => response,
      (error) => {
        if (error.response?.status === 401) {
          window.location.href = '/login';
        }
        const backendError =
          (error.response?.data && (error.response.data.error || error.response.data.message)) ||
          error.message ||
          '请求失败，请检查后端服务';
        message.error(backendError);
        return Promise.reject(error);
      }
    );
  }

  setBaseUrl(baseURL: string) {
    this.baseUrl = deriveApiBaseUrl(baseURL);
    this.client.defaults.baseURL = this.baseUrl;
  }

  // Task APIs
  async listTasks(filter?: TaskFilter): Promise<Task[]> {
    const response = await this.client.get<Task[]>('/api/tasks', { params: filter });
    return response.data;
  }

  async getTask(id: string): Promise<Task> {
    const response = await this.client.get<Task>(`/api/tasks/${id}`);
    return response.data;
  }

  async createTask(request: CreateTaskRequest): Promise<Task> {
    const response = await this.client.post<Task>('/api/tasks', request);
    return response.data;
  }

  async startTask(id: string): Promise<void> {
    await this.client.post(`/api/tasks/${id}/start`);
  }

  async pauseTask(id: string): Promise<void> {
    await this.client.post(`/api/tasks/${id}/pause`);
  }

  async resumeTask(id: string): Promise<void> {
    await this.client.post(`/api/tasks/${id}/resume`);
  }

  async cancelTask(id: string): Promise<void> {
    await this.client.post(`/api/tasks/${id}/cancel`);
  }

  async deleteTask(id: string): Promise<void> {
    await this.client.delete(`/api/tasks/${id}`);
  }

  async retryTask(id: string): Promise<Task> {
    const response = await this.client.post<Task>(`/api/tasks/${id}/retry`);
    return response.data;
  }

  // Metrics APIs
  async getMetrics(query?: MetricsQuery): Promise<MetricsReport> {
    const response = await this.client.get<MetricsReport>('/api/metrics', { params: query });
    return response.data;
  }

  async getTaskStatistics(): Promise<any> {
    const response = await this.client.get('/api/metrics/statistics');
    return response.data;
  }

  // Chat APIs
  async sendChatMessage(message: string): Promise<any> {
    const response = await this.client.post('/api/chat', { content: message });
    return response.data;
  }

  // Health check
  async healthCheck(): Promise<boolean> {
    try {
      await this.client.get('/health');
      return true;
    } catch {
      return false;
    }
  }
}

export const apiClient = new ApiClient();
export default apiClient;

function deriveApiBaseUrl(baseURL?: string): string | undefined {
  const candidates = [
    baseURL,
    import.meta.env.VITE_BACKEND_URL,
    typeof window !== 'undefined' ? window.location.origin : undefined,
    soulbrowserAPI.getBaseUrl(),
  ];

  for (const candidate of candidates) {
    const trimmed = candidate?.trim();
    if (trimmed) {
      return trimmed.replace(/\/+$/, '');
    }
  }

  return undefined;
}
