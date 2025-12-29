import { Card, Steps, Tag, Space, Typography, List, Select, Input } from 'antd';
import {
  ClockCircleOutlined,
  CheckCircleOutlined,
  WarningOutlined,
  PlayCircleOutlined,
} from '@ant-design/icons';
import type { TaskPlan, TaskPlanStep } from '@/types';
import styles from './TaskPlanCard.module.css';

const { Paragraph, Text } = Typography;

type DeliverField = 'schema' | 'artifact_label' | 'filename' | 'source_step_id';
type StageCoverageStatus = 'existing' | 'auto_strategy' | 'placeholder' | 'missing';
type StageDisplayStatus = StageCoverageStatus | 'unknown';

const SCHEMA_OPTIONS = [
  { value: 'generic_observation_v1', label: 'Generic Observation' },
  { value: 'market_info_v1', label: 'Market Info' },
  { value: 'news_brief_v1', label: 'News Brief' },
  { value: 'github_repos_v1', label: 'GitHub Repos' },
  { value: 'twitter_feed_v1', label: 'Twitter Feed' },
  { value: 'facebook_feed_v1', label: 'Facebook Feed' },
  { value: 'linkedin_profile_v1', label: 'LinkedIn Profile' },
  { value: 'hackernews_feed_v1', label: 'Hacker News Feed' },
];

const DELIVER_FIELD_LABELS: Record<DeliverField, string> = {
  schema: 'Schema',
  artifact_label: 'Artifact Label',
  filename: '文件名',
  source_step_id: '来源 Step',
};

const STAGE_SEQUENCE = [
  { key: 'navigate', label: '导航' },
  { key: 'observe', label: '观察' },
  { key: 'act', label: '执行' },
  { key: 'parse', label: '解析' },
  { key: 'deliver', label: '交付' },
];

const STAGE_STATUS_LABEL: Record<StageDisplayStatus, string> = {
  existing: '计划覆盖',
  auto_strategy: '自动补齐',
  placeholder: '占位补齐',
  missing: '仍缺失',
  unknown: '待检测',
};

const STAGE_STATUS_CLASS: Record<StageDisplayStatus, keyof typeof styles> = {
  existing: 'stageChipExisting',
  auto_strategy: 'stageChipAuto',
  placeholder: 'stageChipPlaceholder',
  missing: 'stageChipMissing',
  unknown: 'stageChipUnknown',
};

const COVERAGE_STATUS_WEIGHT: StageCoverageStatus[] = ['existing', 'auto_strategy', 'placeholder', 'missing'];
const STAGE_LABEL_MAP = STAGE_SEQUENCE.reduce<Record<string, string>>((acc, entry) => {
  acc[entry.key] = entry.label;
  return acc;
}, {});

interface Props {
  plan: TaskPlan;
  className?: string;
  editable?: boolean;
  onPlanChange?: (plan: TaskPlan) => void;
}

interface CustomToolConfig {
  name?: string;
  payload?: Record<string, unknown>;
}

export default function TaskPlanCard({ plan, className, editable = false, onPlanChange }: Props) {
  if (!plan) {
    return null;
  }

  const steps = plan.steps ?? [];
  const rationale = plan.meta?.rationale ?? [];
  const riskAssessment = plan.meta?.risk_assessment ?? [];
  const overlays = plan.meta?.overlays ?? plan.overlays ?? [];
  const overlayItems = Array.isArray(overlays) ? overlays : [];
  const stageCoverage = buildStageCoverage(overlayItems);
  const hasStageSummary = Object.values(stageCoverage).some((entry) => entry !== null);
  const stageLogs = overlayItems.filter(shouldDisplayStageLog);
  const stageLabelMap = STAGE_LABEL_MAP;

  const handleDeliverFieldChange = (stepId: string, field: DeliverField, value: string) => {
    if (!editable || !onPlanChange) {
      return;
    }
    const nextPlan: TaskPlan = {
      ...plan,
      steps: steps.map((step) => {
        if (step.id !== stepId) {
          return step;
        }
        return updateDeliverPayload(step, field, value);
      }),
    };
    onPlanChange(nextPlan);
  };

  return (
    <Card className={`${styles.planCard} ${className ?? ''}`} bordered>
      <div className={styles.planHeader}>
        <h3>{plan.title || '任务执行计划'}</h3>
        {plan.description && <Paragraph className={styles.description}>{plan.description}</Paragraph>}
        <Space>
          <Tag icon={<ClockCircleOutlined />}>步骤数: {steps.length}</Tag>
          {riskAssessment.length > 0 && <Tag color="warning">存在风险提示</Tag>}
        </Space>
      </div>

      {hasStageSummary && (
        <div className={styles.stageSummary}>
          {STAGE_SEQUENCE.map((stage) => {
            const entry = stageCoverage[stage.key];
            const status: StageDisplayStatus = entry?.status ?? 'unknown';
            const chipClassName = `${styles.stageChip} ${styles[STAGE_STATUS_CLASS[status]] ?? ''}`;
            const message = entry?.message;
            return (
              <div key={stage.key} className={chipClassName} title={message || stage.label}>
                <span className={styles.stageLabel}>{stage.label}</span>
                <span className={styles.stageState}>{STAGE_STATUS_LABEL[status]}</span>
                {message && <span className={styles.stageMessage}>{message}</span>}
              </div>
            );
          })}
        </div>
      )}

      <div className={styles.planSteps}>
        <Steps
          direction="vertical"
          size="small"
          items={steps.map((step, index) => {
            const toolName = getToolName(step.tool);
            const toolDetail = formatToolDetail(step.tool);
            const customConfig = getCustomConfig(step);
            const deliverName = customConfig?.name?.toLowerCase();
            const isDeliverStep = deliverName === 'data.deliver.structured';
            const deliverPayload = isDeliverStep ? extractDeliverPayload(step) : null;

            return {
              title: step.title || `步骤 ${index + 1}`,
              description: (
                <div>
                  {step.detail && <div>{step.detail}</div>}
                  <div className={styles.stepDetails}>
                    <Tag bordered={false}>工具: {toolName}</Tag>
                    {toolDetail && <Tag bordered={false}>{toolDetail}</Tag>}
                    {step.tool?.wait && <Tag bordered={false}>等待: {step.tool.wait}</Tag>}
                    {typeof step.tool?.timeout_ms === 'number' && (
                      <Tag bordered={false}>超时: {step.tool.timeout_ms}ms</Tag>
                    )}
                    {isDeliverStep &&
                      (Object.keys(DELIVER_FIELD_LABELS) as DeliverField[]).map((field) => (
                        <Tag
                          key={`${step.id}-${field}`}
                          color={deliverPayload?.[field] ? 'processing' : 'error'}
                          bordered={false}
                        >
                          {DELIVER_FIELD_LABELS[field]}: {deliverPayload?.[field] || '未填写'}
                        </Tag>
                      ))}
                  </div>
                  {editable && isDeliverStep && (
                    <div className={styles.deliverEditor}>
                      <Text strong>补充交付信息</Text>
                      <div className={styles.deliverField}>
                        <span className={styles.fieldLabel}>Schema</span>
                        <Select
                          placeholder="选择交付 Schema"
                          options={SCHEMA_OPTIONS}
                          value={deliverPayload?.schema || undefined}
                          onChange={(value) => handleDeliverFieldChange(step.id, 'schema', value ?? '')}
                          showSearch
                          allowClear
                        />
                      </div>
                      <div className={styles.deliverFieldGroup}>
                        <div className={styles.deliverField}>
                          <span className={styles.fieldLabel}>Artifact Label</span>
                          <Input
                            value={deliverPayload?.artifact_label || ''}
                            placeholder="structured.output_label"
                            onChange={(event) =>
                              handleDeliverFieldChange(step.id, 'artifact_label', event.target.value)
                            }
                          />
                        </div>
                        <div className={styles.deliverField}>
                          <span className={styles.fieldLabel}>文件名</span>
                          <Input
                            value={deliverPayload?.filename || ''}
                            placeholder="result.json"
                            onChange={(event) =>
                              handleDeliverFieldChange(step.id, 'filename', event.target.value)
                            }
                          />
                        </div>
                        <div className={styles.deliverField}>
                          <span className={styles.fieldLabel}>来源 Step</span>
                          <Input
                            value={deliverPayload?.source_step_id || ''}
                            placeholder="解析步骤 ID"
                            onChange={(event) =>
                              handleDeliverFieldChange(step.id, 'source_step_id', event.target.value)
                            }
                          />
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              ),
              icon:
                index === 0 ? (
                  <PlayCircleOutlined />
                ) : index === steps.length - 1 ? (
                  <CheckCircleOutlined />
                ) : (
                  <ClockCircleOutlined />
                ),
            };
          })}
        />
      </div>

      {stageLogs.length > 0 && (
        <div className={styles.stageLog}>
          <Text strong>阶段与策略</Text>
          <List
            size="small"
            dataSource={stageLogs}
            renderItem={(item) => (
              <List.Item>
                <Tag color="blue">{stageLabelMap[item.stage?.toLowerCase()] ?? item.stage}</Tag>
                <span>{item.message || item.detail}</span>
                {item.strategy && (
                  <Tag bordered={false} color="default" style={{ marginLeft: 8 }}>
                    策略: {item.strategy}
                  </Tag>
                )}
              </List.Item>
            )}
          />
        </div>
      )}

      {(rationale.length > 0 || riskAssessment.length > 0) && (
        <div className={styles.planMeta}>
          {rationale.length > 0 && (
            <div>
              <Text strong>规划依据</Text>
              <List size="small" dataSource={rationale} renderItem={(item) => <List.Item>{item}</List.Item>} />
            </div>
          )}
          {riskAssessment.length > 0 && (
            <div className={styles.riskBlock}>
              <Text strong>风险提示</Text>
              <List
                size="small"
                dataSource={riskAssessment}
                renderItem={(item) => (
                  <List.Item>
                    <WarningOutlined style={{ color: '#faad14', marginRight: 6 }} />
                    {item}
                  </List.Item>
                )}
              />
            </div>
          )}
        </div>
      )}

      <Space className={styles.planActions}>
        <Tag color="blue">计划已自动执行</Tag>
      </Space>
    </Card>
  );
}

function getToolName(tool?: TaskPlanStep['tool']) {
  if (!tool?.kind) {
    return '未知工具';
  }
  return Object.keys(tool.kind)[0] || '自定义';
}

function buildStageCoverage(overlays: any[]) {
  const coverage = STAGE_SEQUENCE.reduce<Record<string, { status: StageCoverageStatus; message?: string } | null>>(
    (acc, entry) => {
      acc[entry.key] = null;
      return acc;
    },
    {}
  );

  overlays.forEach((item) => {
    const stage = typeof item?.stage === 'string' ? item.stage.toLowerCase() : '';
    const status = typeof item?.status === 'string' ? (item.status as StageCoverageStatus) : undefined;
    if (!stage || !(stage in coverage)) {
      return;
    }
    if (!status || !isCoverageStatus(status)) {
      return;
    }
    if (coverage[stage]) {
      const existingIndex = COVERAGE_STATUS_WEIGHT.indexOf(coverage[stage]!.status);
      const nextIndex = COVERAGE_STATUS_WEIGHT.indexOf(status);
      if (nextIndex >= existingIndex) {
        return;
      }
    }
    coverage[stage] = {
      status,
      message: item?.detail || item?.message || undefined,
    };
  });

  return coverage;
}

function isCoverageStatus(value: string): value is StageCoverageStatus {
  return COVERAGE_STATUS_WEIGHT.includes(value as StageCoverageStatus);
}

function shouldDisplayStageLog(item: any) {
  if (!item || typeof item.stage !== 'string') {
    return false;
  }
  const message = item.message || item.detail;
  if (typeof message !== 'string') {
    return false;
  }
  if (typeof item.status === 'string' && isCoverageStatus(item.status)) {
    return false;
  }
  return true;
}

function formatToolDetail(tool?: TaskPlanStep['tool']) {
  if (!tool?.kind) {
    return null;
  }
  const name = getToolName(tool);
  const raw = tool.kind[name];
  if (!raw || typeof raw !== 'object') {
    return null;
  }
  const payload = raw as Record<string, unknown>;

  if (name === 'Navigate' && typeof payload.url === 'string') {
    return `URL: ${payload.url}`;
  }
  if (name === 'TypeText' && typeof payload.text === 'string') {
    return `输入: ${payload.text}`;
  }
  if (name === 'Click' && payload.locator) {
    return `定位: ${JSON.stringify(payload.locator)}`;
  }
  if (name === 'Custom') {
    const customName = typeof payload.name === 'string' ? payload.name : '自定义工具';
    if (customName === 'data.deliver.structured') {
      const deliverPayload = payload.payload as Record<string, unknown> | undefined;
      const schema =
        deliverPayload && typeof deliverPayload.schema === 'string'
          ? deliverPayload.schema
          : '未设置 schema';
      return `${customName} (${schema})`;
    }
    return customName;
  }

  return JSON.stringify(payload);
}

function getCustomConfig(step: TaskPlanStep): CustomToolConfig | null {
  const kind = step.tool?.kind ?? {};
  const rawCustom = (kind as Record<string, unknown>).Custom as CustomToolConfig | undefined;
  if (!rawCustom || typeof rawCustom !== 'object') {
    return null;
  }
  return rawCustom;
}

function extractDeliverPayload(
  step: TaskPlanStep
): Partial<Record<DeliverField, string>> | null {
  const custom = getCustomConfig(step);
  if (!custom) {
    return null;
  }
  const payload = (custom.payload as Record<string, unknown>) || {};
  return {
    schema: typeof payload.schema === 'string' ? payload.schema : undefined,
    artifact_label: typeof payload.artifact_label === 'string' ? payload.artifact_label : undefined,
    filename: typeof payload.filename === 'string' ? payload.filename : undefined,
    source_step_id:
      typeof payload.source_step_id === 'string' ? payload.source_step_id : undefined,
  } satisfies Partial<Record<DeliverField, string>>;
}

function updateDeliverPayload(step: TaskPlanStep, field: DeliverField, value: string): TaskPlanStep {
  const tool = step.tool ?? { kind: {} };
  const kind = tool.kind ?? {};
  const rawCustom = (kind as Record<string, unknown>).Custom as CustomToolConfig | undefined;
  if (!rawCustom) {
    return step;
  }
  const existingPayload = (rawCustom.payload as Record<string, any>) ?? {};
  const nextPayload = { ...existingPayload, [field]: value };
  const nextCustom = { ...rawCustom, payload: nextPayload };
  const nextKind = { ...kind, Custom: nextCustom };
  return {
    ...step,
    tool: {
      ...tool,
      kind: nextKind,
    },
  };
}
