/**
 * Task-related type definitions
 */

export type TaskStatus = 'pending' | 'running' | 'paused' | 'completed' | 'failed' | 'cancelled';

export interface Task {
  id: string;
  name: string;
  description?: string;
  status: TaskStatus;
  progress: number; // 0-100
  currentStep?: string;
  totalSteps: number;
  completedSteps: number;
  startTime?: Date;
  endTime?: Date;
  duration?: number; // in seconds
  error?: TaskError;
  result?: any;
  plan?: TaskPlan;
  metadata?: Record<string, any>;
}

export interface TaskError {
  code: string;
  message: string;
  details?: string;
  timestamp: Date;
  retryable: boolean;
}

export interface TaskPlan {
  id?: string;
  task_id?: string;
  title: string;
  description?: string;
  created_at?: string;
  meta?: TaskPlanMeta;
  overlays?: any[];
  steps: TaskPlanStep[];
}

export interface TaskPlanMeta {
  rationale?: string[];
  risk_assessment?: string[];
  vendor_context?: Record<string, unknown>;
}

export interface TaskPlanStep {
  id: string;
  title: string;
  detail?: string;
  metadata?: Record<string, unknown>;
  requires_approval?: boolean;
  validations?: Record<string, unknown>[];
  tool: TaskPlanTool;
}

export interface TaskPlanTool {
  kind: Record<string, unknown>;
  timeout_ms?: number | null;
  wait?: string | null;
}

export interface ElementLocator {
  primary: LocatorStrategy;
  fallback?: LocatorStrategy[];
  confidence: number; // 0-1
}

export interface LocatorStrategy {
  type: 'css' | 'xpath' | 'text' | 'visual';
  value: string;
  confidence: number;
}

export interface StepValidation {
  condition: string;
  expectedResult: any;
  timeout: number; // in milliseconds
}

export interface RetryStrategy {
  maxRetries: number;
  interval: number; // in milliseconds
  backoffMultiplier: number;
}

export interface PolicyCheck {
  policyId: string;
  policyName: string;
  passed: boolean;
  message?: string;
  severity: 'info' | 'warning' | 'error';
}

export interface CreateTaskRequest {
  name: string;
  description?: string;
  prompt?: string; // For natural language task creation
  parameters?: Record<string, any>;
  templateId?: string; // For template-based creation
}

export interface TaskUpdate {
  id: string;
  status?: TaskStatus;
  progress?: number;
  currentStep?: string;
  completedSteps?: number;
  error?: TaskError;
  timestamp: Date;
}

export interface TaskFilter {
  status?: TaskStatus[];
  search?: string;
  startDate?: Date;
  endDate?: Date;
  tags?: string[];
}

export interface TaskStatistics {
  total: number;
  pending: number;
  running: number;
  completed: number;
  failed: number;
  successRate: number;
  averageDuration: number;
}
