import { Card, Steps, Tag, Button, Space, Collapse } from 'antd';
import {
  ClockCircleOutlined,
  CheckCircleOutlined,
  WarningOutlined,
  PlayCircleOutlined,
} from '@ant-design/icons';
import type { TaskPlan } from '@/types';
import { formatDuration } from '@/utils/format';
import { useTaskStore } from '@/stores/taskStore';
import styles from './TaskPlanCard.module.css';

const { Panel } = Collapse;

interface Props {
  plan: TaskPlan;
  className?: string;
}

export default function TaskPlanCard({ plan, className }: Props) {
  const createTask = useTaskStore((state) => state.createTask);

  const handleExecute = async () => {
    try {
      const task = await createTask(plan.id, '执行任务计划');
      // Navigate to task page or start execution
      console.log('Task created:', task);
    } catch (error) {
      console.error('Failed to create task:', error);
    }
  };

  const getRiskColor = (level: string) => {
    switch (level) {
      case 'low':
        return 'success';
      case 'medium':
        return 'warning';
      case 'high':
        return 'error';
      default:
        return 'default';
    }
  };

  return (
    <Card className={`${styles.planCard} ${className}`} bordered>
      <div className={styles.planHeader}>
        <h3>任务执行计划</h3>
        <Space>
          <Tag icon={<ClockCircleOutlined />}>
            预计耗时: {formatDuration(plan.estimatedDuration)}
          </Tag>
          <Tag color={getRiskColor(plan.riskLevel)}>
            风险等级: {plan.riskLevel === 'low' ? '低' : plan.riskLevel === 'medium' ? '中' : '高'}
          </Tag>
          <Tag>成功率: {(plan.successProbability * 100).toFixed(0)}%</Tag>
        </Space>
      </div>

      <div className={styles.planSteps}>
        <Steps
          direction="vertical"
          size="small"
          items={plan.steps.map((step) => ({
            title: step.name,
            description: (
              <div>
                <div>{step.description}</div>
                {step.locator && (
                  <div className={styles.stepDetails}>
                    <Tag size="small">工具: {step.tool}</Tag>
                    <Tag size="small">
                      定位: {step.locator.primary.type} - {step.locator.primary.value}
                    </Tag>
                    <Tag size="small">
                      置信度: {(step.locator.confidence * 100).toFixed(0)}%
                    </Tag>
                  </div>
                )}
              </div>
            ),
            icon:
              step.status === 'completed' ? (
                <CheckCircleOutlined />
              ) : (
                <ClockCircleOutlined />
              ),
          }))}
        />
      </div>

      {plan.policyChecks.length > 0 && (
        <div className={styles.policyChecks}>
          <h4>策略检查</h4>
          {plan.policyChecks.map((check) => (
            <div
              key={check.policyId}
              className={`${styles.policyCheck} ${
                check.passed ? styles.passed : styles.failed
              }`}
            >
              {check.passed ? (
                <CheckCircleOutlined style={{ color: '#52c41a' }} />
              ) : (
                <WarningOutlined style={{ color: '#ff4d4f' }} />
              )}
              <span>{check.policyName}</span>
              {check.message && <span className={styles.checkMessage}>{check.message}</span>}
            </div>
          ))}
        </div>
      )}

      <div className={styles.planActions}>
        <Button type="primary" icon={<PlayCircleOutlined />} onClick={handleExecute} size="large">
          开始执行
        </Button>
        <Button>查看详情</Button>
        <Button>修改计划</Button>
      </div>
    </Card>
  );
}
