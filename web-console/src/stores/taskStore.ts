/**
 * Task state management store
 */

import { message } from 'antd';
import { create } from 'zustand';
import { immer } from 'zustand/middleware/immer';
import type { Task, TaskUpdate, TaskFilter } from '@/types';
import { apiClient } from '@/api/client';
import {
  soulbrowserAPI,
  type TaskSummary,
  type TaskDetailResponse,
  type TaskStatusSnapshot,
} from '@/api/soulbrowser';

interface TaskState {
  // State
  tasks: Map<string, Task>;
  selectedTaskId: string | null;
  filter: TaskFilter;
  loading: boolean;
  error: string | null;

  // Actions
  setTasks: (tasks: Task[]) => void;
  addTask: (task: Task) => void;
  updateTask: (update: TaskUpdate) => void;
  removeTask: (taskId: string) => void;
  selectTask: (taskId: string | null) => void;
  setFilter: (filter: Partial<TaskFilter>) => void;

  // Async actions
  fetchTasks: () => Promise<void>;
  createTask: (name: string, description?: string) => Promise<Task>;
  startTask: (taskId: string) => Promise<void>;
  pauseTask: (taskId: string) => Promise<void>;
  cancelTask: (taskId: string) => Promise<void>;
  retryTask: (taskId: string) => Promise<void>;
  fetchTaskDetail: (taskId: string) => Promise<TaskDetailResponse>;
  fetchTaskExecutions: (taskId: string) => Promise<any[]>;
  fetchTaskStatus: (taskId: string) => Promise<TaskStatusSnapshot>;
}

export const useTaskStore = create<TaskState>()(
  immer((set, get) => ({
    // Initial state
    tasks: new Map(),
    selectedTaskId: null,
    filter: {},
    loading: false,
    error: null,

    // Sync actions
    setTasks: (tasks) =>
      set((state) => {
        state.tasks = new Map(tasks.map((task) => [task.id, task]));
      }),

    addTask: (task) =>
      set((state) => {
        state.tasks = new Map(state.tasks).set(task.id, task);
      }),

    updateTask: (update) =>
      set((state) => {
        const existing = state.tasks.get(update.id);
        if (existing) {
          const updated = { ...existing, ...update } as Task;
          state.tasks = new Map(state.tasks).set(update.id, updated);
        }
      }),

    removeTask: (taskId) =>
      set((state) => {
        const next = new Map(state.tasks);
        next.delete(taskId);
        state.tasks = next;
        if (state.selectedTaskId === taskId) {
            state.selectedTaskId = null;
        }
      }),

    selectTask: (taskId) =>
      set((state) => {
        state.selectedTaskId = taskId;
      }),

    setFilter: (filter) =>
      set((state) => {
        state.filter = { ...state.filter, ...filter };
      }),

    // Async actions
    fetchTasks: async () => {
      set((state) => {
        state.loading = true;
        state.error = null;
      });

      try {
        const summaries = await soulbrowserAPI.listTasks();
        const tasks = summaries.map(mapSummaryToTask);
        get().setTasks(tasks);
      } catch (error) {
        set((state) => {
          state.error = error instanceof Error ? error.message : 'Failed to fetch tasks';
        });
      } finally {
        set((state) => {
          state.loading = false;
        });
      }
    },

    createTask: async (name, description) => {
      const task = await apiClient.createTask({ name, description });
      get().addTask(task);
      return task;
    },

    startTask: async () => {
      message.info('任务控制操作尚未在测试控制台中开放');
    },

    pauseTask: async () => {
      message.info('任务控制操作尚未在测试控制台中开放');
    },

    cancelTask: async () => {
      message.info('任务控制操作尚未在测试控制台中开放');
    },

    retryTask: async () => {
      message.info('任务控制操作尚未在测试控制台中开放');
    },

    fetchTaskDetail: async (taskId) => {
      return await soulbrowserAPI.getTask(taskId);
    },

    fetchTaskExecutions: async (taskId) => {
      return await soulbrowserAPI.getTaskExecutions(taskId);
    },

    fetchTaskStatus: async (taskId) => {
      return await soulbrowserAPI.getTaskStatus(taskId);
    },
  }))
);

// Selectors
export const selectAllTasks = (state: TaskState) => Array.from(state.tasks.values());

export const selectTaskById = (id: string) => (state: TaskState) => state.tasks.get(id);

export const selectTasksByStatus = (status: string) => (state: TaskState) =>
  Array.from(state.tasks.values()).filter((task) => task.status === status);

export const selectSelectedTask = (state: TaskState) =>
  state.selectedTaskId ? state.tasks.get(state.selectedTaskId) : null;

export const selectTaskCount = (state: TaskState) => state.tasks.size;

export const selectRunningTasksCount = (state: TaskState) =>
  Array.from(state.tasks.values()).filter((task) => task.status === 'running').length;

function mapSummaryToTask(summary: TaskSummary): Task {
  return {
    id: summary.task_id,
    name: summary.prompt || summary.task_id,
    description: summary.prompt,
    status: 'completed',
    progress: 100,
    totalSteps: 0,
    completedSteps: 0,
    startTime: summary.created_at ? new Date(summary.created_at) : undefined,
    endTime: summary.created_at ? new Date(summary.created_at) : undefined,
    metadata: {
      source: summary.source,
      planner: summary.planner,
      llm_provider: summary.llm_provider,
      llm_model: summary.llm_model,
      artifact_path: summary.path,
      session_id: summary.session_id,
    },
  };
}
