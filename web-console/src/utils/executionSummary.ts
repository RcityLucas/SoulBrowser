import type { ExecutionSummary } from '@/stores/chatStore';

export interface ExecutionResultEntry {
  label: string;
  data?: unknown;
  artifactPath?: string;
}

export function buildExecutionSummary(
  flow: any,
  success: boolean,
  stdout?: string,
  stderr?: string
): ExecutionSummary | undefined {
  const flowSteps = flow?.execution?.steps;
  const steps: ExecutionSummary['steps'] = Array.isArray(flowSteps)
    ? flowSteps.map((step: any) => ({
        stepId: step.step_id || step.id || 'step',
        title: step.title || step.step_id || '未命名步骤',
        status: step.status || 'unknown',
        attempts: step.attempts ?? 0,
        durationMs: step.total_run_ms ?? 0,
        error: step.error ?? null,
      }))
    : [];

  const artifactPath = Array.isArray(flowSteps)
    ? flowSteps
        .flatMap((step: any) => step.dispatches || [])
        .map((dispatch: any) => dispatch.output?.output?.artifact_path)
        .find(Boolean)
    : undefined;

  const hasStdout = Boolean(stdout?.trim());
  const hasStderr = Boolean(stderr?.trim());

  if (!steps.length && !hasStdout && !hasStderr && !artifactPath) {
    return undefined;
  }

  return {
    success,
    stdout,
    stderr,
    artifactPath,
    steps,
  };
}

export function extractExecutionResults(execution: any): ExecutionResultEntry[] {
  if (!execution || !Array.isArray(execution.steps)) {
    return [];
  }

  const entries: ExecutionResultEntry[] = [];

  execution.steps.forEach((step: any) => {
    const label = step.title || step.step_id || '步骤';
    (step.dispatches || []).forEach((dispatch: any) => {
      const output = dispatch.output?.output;
      if (!output) {
        return;
      }
      if (output.result) {
        entries.push({ label, data: output.result });
      } else if (output.preview) {
        entries.push({ label, data: output.preview });
      } else if (output.observation && entries.length === 0) {
        entries.push({ label, data: output.observation });
      }
      if (output.artifact_path) {
        entries.push({ label: `${label} 产物`, artifactPath: output.artifact_path });
      }
    });
  });

  return entries;
}
