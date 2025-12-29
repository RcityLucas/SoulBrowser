/**
 * Task management hook
 */

import { useEffect } from 'react';
import { useTaskStore } from '@/stores/taskStore';

export function useTasks() {
  const {
    tasks,
    selectedTaskId,
    filter,
    loading,
    error,
    fetchTasks,
    selectTask,
    setFilter,
    createTask,
    startTask,
    pauseTask,
    cancelTask,
    retryTask,
    fetchTaskDetail,
    fetchTaskExecutions,
    fetchTaskStatus,
  } = useTaskStore();

  // Fetch tasks on mount
  useEffect(() => {
    fetchTasks();
  }, [fetchTasks]);

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
    fetchTaskDetail,
    fetchTaskExecutions,
    fetchTaskStatus,
    selectTask,
    setFilter,
  };
}

export default useTasks;
