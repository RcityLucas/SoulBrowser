import { Card, Steps, Tag, Space, Typography, List } from 'antd';
import { ClockCircleOutlined, CheckCircleOutlined, WarningOutlined, PlayCircleOutlined } from '@ant-design/icons';
import type { TaskPlan } from '@/types';
import styles from './TaskPlanCard.module.css';

interface Props {
  plan: TaskPlan;
  className?: string;
}

export default function TaskPlanCard({ plan, className }: Props) {
  if (!plan) {
    return null;
  }

  const rationale = plan.meta?.rationale ?? [];
  const riskAssessment = plan.meta?.risk_assessment ?? [];

  const getToolName = (tool?: TaskPlan['steps'][number]['tool']) =>
    tool?.kind ? Object.keys(tool.kind)[0] : 'Custom';

  const formatToolDetail = (tool?: TaskPlan['steps'][number]['tool']) => {
    if (!tool?.kind) {
      return null;
    }
    const name = getToolName(tool);
    const payload = tool.kind[name];
    if (!payload || typeof payload !== 'object') {
      return null;
    }
    if (name === 'Navigate') {
      return `URL: ${payload.url}`;
    }
    if (name === 'TypeText') {
      return `输入: ${payload.text}`;
    }
    if (name === 'Click' && payload.locator) {
      return `定位: ${JSON.stringify(payload.locator)}`;
    }
    if (name === 'Custom') {
      return payload.name || '自定义工具';
    }
    return JSON.stringify(payload);
  };

  return (
    <Card className={`${styles.planCard} ${className}`} bordered>
      <div className={styles.planHeader}>
        <h3>{plan.title || '任务执行计划'}</h3>
        {plan.description && (
          <Typography.Paragraph className={styles.description}>
            {plan.description}
          </Typography.Paragraph>
        )}
        <Space>
          <Tag icon={<ClockCircleOutlined />}>
            步骤数: {plan.steps?.length ?? 0}
          </Tag>
          {riskAssessment.length > 0 && (
            <Tag color="warning">存在风险提示</Tag>
          )}
        </Space>
      </div>

      <div className={styles.planSteps}>
        <Steps
          direction="vertical"
          size="small"
          items={(plan.steps ?? []).map((step, index) => ({
            title: step.title || `步骤 ${index + 1}`,
            description: (
              <div>
                {step.detail && <div>{step.detail}</div>}
                <div className={styles.stepDetails}>
                  <Tag size="small">工具: {getToolName(step.tool)}</Tag>
                  {formatToolDetail(step.tool) && (
                    <Tag size="small">{formatToolDetail(step.tool)}</Tag>
                  )}
                  {step.tool?.wait && <Tag size="small">等待: {step.tool.wait}</Tag>}
                  {typeof step.tool?.timeout_ms === 'number' && (
                    <div className={styles.stepDetails}>
                      <Tag size="small">超时: {step.tool.timeout_ms}ms</Tag>
                    </div>
                  )}
                </div>
              </div>
            ),
            icon:
              index === 0 ? (
                <PlayCircleOutlined />
              ) : index === (plan.steps?.length ?? 1) - 1 ? (
                <CheckCircleOutlined />
              ) : (
                <ClockCircleOutlined />
              ),
          }))}
        />
      </div>

      {(rationale.length > 0 || riskAssessment.length > 0) && (
        <div className={styles.planMeta}>
          {rationale.length > 0 && (
            <div>
              <Typography.Text strong>规划依据</Typography.Text>
              <List
                size="small"
                dataSource={rationale}
                renderItem={(item) => <List.Item>{item}</List.Item>}
              />
            </div>
          )}
          {riskAssessment.length > 0 && (
            <div className={styles.riskBlock}>
              <Typography.Text strong>风险提示</Typography.Text>
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
