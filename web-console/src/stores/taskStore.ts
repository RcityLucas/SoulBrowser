/**
 * Task state management store
 */

import { create } from 'zustand';
import { immer } from 'zustand/middleware/immer';
import type { Task, TaskUpdate, TaskFilter } from '@/types';
import { apiClient } from '@/api/client';

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
        state.tasks.clear();
        tasks.forEach((task) => state.tasks.set(task.id, task));
      }),

    addTask: (task) =>
      set((state) => {
        state.tasks.set(task.id, task);
      }),

    updateTask: (update) =>
      set((state) => {
        const task = state.tasks.get(update.id);
        if (task) {
          Object.assign(task, update);
          state.tasks.set(update.id, task);
        }
      }),

    removeTask: (taskId) =>
      set((state) => {
        state.tasks.delete(taskId);
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
        const tasks = await apiClient.listTasks(get().filter);
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

    startTask: async (taskId) => {
      await apiClient.startTask(taskId);
      set((state) => {
        const task = state.tasks.get(taskId);
        if (task) {
          task.status = 'running';
        }
      });
    },

    pauseTask: async (taskId) => {
      await apiClient.pauseTask(taskId);
      set((state) => {
        const task = state.tasks.get(taskId);
        if (task) {
          task.status = 'paused';
        }
      });
    },

    cancelTask: async (taskId) => {
      await apiClient.cancelTask(taskId);
      set((state) => {
        const task = state.tasks.get(taskId);
        if (task) {
          task.status = 'cancelled';
        }
      });
    },

    retryTask: async (taskId) => {
      const newTask = await apiClient.retryTask(taskId);
      get().addTask(newTask);
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
