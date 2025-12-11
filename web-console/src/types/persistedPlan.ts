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
  planner: string;
  llm_provider?: string | null;
  llm_model?: string | null;
  context_snapshot?: any;
}
