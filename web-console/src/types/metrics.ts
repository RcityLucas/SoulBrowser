/**
 * Metrics and monitoring type definitions
 */

export interface MetricsData {
  timestamp: Date;
  taskMetrics: TaskMetrics;
  performanceMetrics: PerformanceMetrics;
  errorMetrics: ErrorMetrics;
}

export interface TaskMetrics {
  total: number;
  pending: number;
  running: number;
  completed: number;
  failed: number;
  successRate: number; // 0-100
  averageDuration: number; // in seconds
  throughput: number; // tasks per hour
}

export interface PerformanceMetrics {
  avgResponseTime: number; // in ms
  p95ResponseTime: number;
  p99ResponseTime: number;
  cpuUsage: number; // 0-100
  memoryUsage: number; // in MB
  activeConnections: number;
}

export interface ErrorMetrics {
  total: number;
  byType: Record<string, number>;
  recentErrors: ErrorSummary[];
}

export interface ErrorSummary {
  id: string;
  type: string;
  message: string;
  count: number;
  lastOccurrence: Date;
  severity: 'low' | 'medium' | 'high' | 'critical';
}

export interface TimeSeriesData {
  timestamp: Date;
  value: number;
}

export interface MetricsReport {
  period: TimePeriod;
  summary: MetricsSummary;
  timeSeries: {
    successRate: TimeSeriesData[];
    taskCount: TimeSeriesData[];
    avgDuration: TimeSeriesData[];
    errorRate: TimeSeriesData[];
  };
}

export interface TimePeriod {
  start: Date;
  end: Date;
  duration: number; // in seconds
}

export interface MetricsSummary {
  totalTasks: number;
  successfulTasks: number;
  failedTasks: number;
  avgDuration: number;
  medianDuration: number;
  p95Duration: number;
  successRate: number;
  errorRate: number;
  topErrors: ErrorSummary[];
}

export interface MetricsQuery {
  startTime: Date;
  endTime: Date;
  granularity?: 'minute' | 'hour' | 'day';
  metrics?: string[];
  filters?: Record<string, any>;
}
