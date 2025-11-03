/**
 * Metrics state management store
 */

import { create } from 'zustand';
import { immer } from 'zustand/middleware/immer';
import type { MetricsData, MetricsReport, TimeSeriesData } from '@/types';
import { apiClient } from '@/api/client';

interface MetricsState {
  // State
  currentMetrics: MetricsData | null;
  report: MetricsReport | null;
  successRateHistory: TimeSeriesData[];
  taskCountHistory: TimeSeriesData[];
  loading: boolean;
  error: string | null;

  // Actions
  setCurrentMetrics: (metrics: MetricsData) => void;
  setReport: (report: MetricsReport) => void;
  addDataPoint: (type: 'successRate' | 'taskCount', data: TimeSeriesData) => void;

  // Async actions
  fetchMetrics: () => Promise<void>;
  fetchReport: (startTime: Date, endTime: Date) => Promise<void>;
}

export const useMetricsStore = create<MetricsState>()(
  immer((set, get) => ({
    // Initial state
    currentMetrics: null,
    report: null,
    successRateHistory: [],
    taskCountHistory: [],
    loading: false,
    error: null,

    // Sync actions
    setCurrentMetrics: (metrics) =>
      set((state) => {
        state.currentMetrics = metrics;
      }),

    setReport: (report) =>
      set((state) => {
        state.report = report;
        state.successRateHistory = report.timeSeries.successRate;
        state.taskCountHistory = report.timeSeries.taskCount;
      }),

    addDataPoint: (type, data) =>
      set((state) => {
        if (type === 'successRate') {
          state.successRateHistory.push(data);
          // Keep only last 100 points
          if (state.successRateHistory.length > 100) {
            state.successRateHistory.shift();
          }
        } else {
          state.taskCountHistory.push(data);
          if (state.taskCountHistory.length > 100) {
            state.taskCountHistory.shift();
          }
        }
      }),

    // Async actions
    fetchMetrics: async () => {
      set((state) => {
        state.loading = true;
        state.error = null;
      });

      try {
        const report = await apiClient.getMetrics();
        get().setReport(report);
      } catch (error) {
        set((state) => {
          state.error = error instanceof Error ? error.message : 'Failed to fetch metrics';
        });
      } finally {
        set((state) => {
          state.loading = false;
        });
      }
    },

    fetchReport: async (startTime, endTime) => {
      set((state) => {
        state.loading = true;
        state.error = null;
      });

      try {
        const report = await apiClient.getMetrics({ startTime, endTime });
        get().setReport(report);
      } catch (error) {
        set((state) => {
          state.error = error instanceof Error ? error.message : 'Failed to fetch report';
        });
      } finally {
        set((state) => {
          state.loading = false;
        });
      }
    },
  }))
);

// Selectors
export const selectSuccessRate = (state: MetricsState) =>
  state.currentMetrics?.taskMetrics.successRate ?? 0;

export const selectTotalTasks = (state: MetricsState) =>
  state.currentMetrics?.taskMetrics.total ?? 0;

export const selectRunningTasks = (state: MetricsState) =>
  state.currentMetrics?.taskMetrics.running ?? 0;
