import { Card, Tag, List, Typography, Space, Alert, Button } from 'antd';
import { CheckCircleTwoTone, CloseCircleTwoTone } from '@ant-design/icons';
import type { ExecutionSummary } from '@/stores/chatStore';
import styles from './ExecutionSummaryCard.module.css';

interface Props {
  summary: ExecutionSummary;
  onTriggerRemedy?: (action: 'capture' | 'summarize') => void;
}

export default function ExecutionSummaryCard({ summary, onTriggerRemedy }: Props) {
  const hasStdout = Boolean(summary.stdout?.trim());
  const hasStderr = Boolean(summary.stderr?.trim());
  const missingResult = summary.success && summary.missingUserResult;

  return (
    <Card size="small" className={styles.summaryCard} bordered>
      <Space align="center" className={styles.header}>
        {summary.success ? (
          <CheckCircleTwoTone twoToneColor="#52c41a" />
        ) : (
          <CloseCircleTwoTone twoToneColor="#ff4d4f" />
        )}
        <Tag color={summary.success ? 'success' : 'error'}>
          {summary.success ? '执行成功' : '执行失败'}
        </Tag>
        {missingResult && <Tag color="gold">缺少可读答案</Tag>}
        {summary.artifactPath && <span className={styles.artifact}>产出: {summary.artifactPath}</span>}
      </Space>

      {missingResult && (
        <Alert
          type="warning"
          showIcon
          className={styles.missingAlert}
          message="执行成功但未生成可读答案"
          description={
            onTriggerRemedy ? (
              <Space size={8} wrap>
                <Button size="small" type="primary" onClick={() => onTriggerRemedy('capture')}>
                  抓取页面
                </Button>
                <Button size="small" onClick={() => onTriggerRemedy('summarize')}>
                  总结结果
                </Button>
              </Space>
            ) : undefined
          }
        />
      )}

      {(hasStdout || hasStderr) && (
        <div className={styles.logs}>
          {hasStdout && (
            <div className={styles.logBlock}>
              <Typography.Text strong>stdout</Typography.Text>
              <Typography.Paragraph className={styles.logText}>
                {summary.stdout}
              </Typography.Paragraph>
            </div>
          )}
          {hasStderr && (
            <div className={styles.logBlock}>
              <Typography.Text strong type="danger">
                stderr
              </Typography.Text>
              <Typography.Paragraph className={styles.logText} type="danger">
                {summary.stderr}
              </Typography.Paragraph>
            </div>
          )}
        </div>
      )}

      {summary.steps.length > 0 && (
        <List
          size="small"
          header={<div>执行步骤</div>}
          dataSource={summary.steps}
          renderItem={(item) => (
            <List.Item className={styles.stepItem}>
              <Space direction="vertical" size={2}>
                <Space align="center">
                  <Tag color={item.status === 'success' ? 'success' : 'warning'}>
                    {item.status === 'success' ? '成功' : '未完成'}
                  </Tag>
                  <span>{item.title || item.stepId}</span>
                </Space>
                <Typography.Text type="secondary">
                  耗时: {item.durationMs} ms · 尝试: {item.attempts}
                </Typography.Text>
                {item.error && (
                  <Typography.Text type="danger">{item.error}</Typography.Text>
                )}
              </Space>
            </List.Item>
          )}
        />
      )}
    </Card>
  );
}
