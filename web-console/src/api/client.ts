/**
 * HTTP API Client
 */

import axios, { AxiosInstance, AxiosRequestConfig } from 'axios';
import type { Task, CreateTaskRequest, TaskFilter, MetricsQuery, MetricsReport } from '@/types';

class ApiClient {
  private client: AxiosInstance;

  constructor(baseURL: string = '/api') {
    this.client = axios.create({
      baseURL,
      timeout: 30000,
      headers: {
        'Content-Type': 'application/json',
      },
    });

    // Request interceptor
    this.client.interceptors.request.use(
      (config) => {
        // Add auth token if available
        const token = localStorage.getItem('auth_token');
        if (token) {
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
          // Handle unauthorized
          window.location.href = '/login';
        }
        return Promise.reject(error);
      }
    );
  }

  // Task APIs
  async listTasks(filter?: TaskFilter): Promise<Task[]> {
    const response = await this.client.get<Task[]>('/tasks', { params: filter });
    return response.data;
  }

  async getTask(id: string): Promise<Task> {
    const response = await this.client.get<Task>(`/tasks/${id}`);
    return response.data;
  }

  async createTask(request: CreateTaskRequest): Promise<Task> {
    const response = await this.client.post<Task>('/tasks', request);
    return response.data;
  }

  async startTask(id: string): Promise<void> {
    await this.client.post(`/tasks/${id}/start`);
  }

  async pauseTask(id: string): Promise<void> {
    await this.client.post(`/tasks/${id}/pause`);
  }

  async resumeTask(id: string): Promise<void> {
    await this.client.post(`/tasks/${id}/resume`);
  }

  async cancelTask(id: string): Promise<void> {
    await this.client.post(`/tasks/${id}/cancel`);
  }

  async deleteTask(id: string): Promise<void> {
    await this.client.delete(`/tasks/${id}`);
  }

  async retryTask(id: string): Promise<Task> {
    const response = await this.client.post<Task>(`/tasks/${id}/retry`);
    return response.data;
  }

  // Metrics APIs
  async getMetrics(query?: MetricsQuery): Promise<MetricsReport> {
    const response = await this.client.get<MetricsReport>('/metrics', { params: query });
    return response.data;
  }

  async getTaskStatistics(): Promise<any> {
    const response = await this.client.get('/metrics/statistics');
    return response.data;
  }

  // Chat APIs
  async sendChatMessage(message: string): Promise<any> {
    const response = await this.client.post('/chat', { content: message });
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
