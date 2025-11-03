/**
 * Task management hook
 */

import { useEffect } from 'react';
import { useTaskStore } from '@/stores/taskStore';
import { useWebSocket } from './useWebSocket';
import type { TaskUpdate } from '@/types';

export function useTasks() {
  const { send, on } = useWebSocket();
  const {
    tasks,
    selectedTaskId,
    filter,
    loading,
    error,
    fetchTasks,
    addTask,
    updateTask,
    removeTask,
    selectTask,
    setFilter,
    createTask,
    startTask,
    pauseTask,
    cancelTask,
    retryTask,
  } = useTaskStore();

  // Fetch tasks on mount
  useEffect(() => {
    fetchTasks();
  }, [fetchTasks]);

  // Subscribe to task updates via WebSocket
  useEffect(() => {
    const unsubscribeCreated = on('task_created', (task) => {
      addTask(task);
    });

    const unsubscribeUpdated = on('task_updated', (update: TaskUpdate) => {
      updateTask(update);
    });

    const unsubscribeCompleted = on('task_completed', ({ taskId }) => {
      updateTask({
        id: taskId,
        status: 'completed',
        progress: 100,
        timestamp: new Date(),
      });
    });

    const unsubscribeFailed = on('task_failed', ({ taskId, error }) => {
      updateTask({
        id: taskId,
        status: 'failed',
        error,
        timestamp: new Date(),
      });
    });

    return () => {
      unsubscribeCreated();
      unsubscribeUpdated();
      unsubscribeCompleted();
      unsubscribeFailed();
    };
  }, [on, addTask, updateTask]);

  return {
    tasks: Array.from(tasks.values()),
    selectedTask: selectedTaskId ? tasks.get(selectedTaskId) : null,
    filter,
    loading,
    error,
    fetchTasks,
    createTask,
    startTask,
    pauseTask,
    cancelTask,
    retryTask,
    selectTask,
    setFilter,
  };
}

export default useTasks;
