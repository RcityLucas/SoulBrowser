import { useState } from 'react';
import { Card, Table, Tag, Progress, Space, Button, Input, Select, Drawer, Spin } from 'antd';
import {
  PlayCircleOutlined,
  PauseCircleOutlined,
  CloseCircleOutlined,
  ReloadOutlined,
  SearchOutlined,
} from '@ant-design/icons';
import { useTasks } from '@/hooks/useTasks';
import { formatTime, formatDuration } from '@/utils/format';
import type { Task, TaskStatus } from '@/types';
import styles from './TasksPage.module.css';
import TaskPlanCard from '@/components/chat/TaskPlanCard';
import ExecutionSummaryCard from '@/components/chat/ExecutionSummaryCard';
import ExecutionResultCard from '@/components/chat/ExecutionResultCard';
import BackendStatusBar from '@/components/common/BackendStatusBar';
import type { ExecutionSummary } from '@/stores/chatStore';
import { buildExecutionSummary, extractExecutionResults } from '@/utils/executionSummary';
import type { TaskDetailResponse, PersistedPlanRecord } from '@/api/soulbrowser';
import type { TaskPlan } from '@/types';

const { Search } = Input;

export default function TasksPage() {
  const {
    tasks,
    loading,
    startTask,
    pauseTask,
    cancelTask,
    retryTask,
    setFilter,
    fetchTaskDetail,
    fetchTaskExecutions,
  } = useTasks();
  const [detailOpen, setDetailOpen] = useState(false);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailRecord, setDetailRecord] = useState<TaskDetailResponse | null>(null);
  const [detailPlan, setDetailPlan] = useState<TaskPlan | null>(null);
  const [detailSummary, setDetailSummary] = useState<ExecutionSummary | undefined>();
  const [detailResults, setDetailResults] = useState<ReturnType<typeof extractExecutionResults>>([]);

  const handleViewDetail = async (taskId: string) => {
    setDetailOpen(true);
    setDetailLoading(true);
    try {
      const detail = await fetchTaskDetail(taskId);
      setDetailRecord(detail);
      setDetailPlan(normalizePlan(detail.task));

      try {
        const executions = await fetchTaskExecutions(taskId);
        const latestExecution = executions[executions.length - 1];
        const summary = buildExecutionSummary(
          { execution: latestExecution },
          latestExecution?.success ?? true,
          undefined,
          undefined
        );
        setDetailSummary(summary);
        setDetailResults(extractExecutionResults(latestExecution));
      } catch (executionErr) {
        console.warn('No execution data for task', taskId, executionErr);
        setDetailSummary(undefined);
        setDetailResults([]);
      }
    } catch (error) {
      console.error('Failed to load task detail', error);
    } finally {
      setDetailLoading(false);
    }
  };

  const getStatusColor = (status: TaskStatus) => {
    switch (status) {
      case 'running':
        return 'processing';
      case 'completed':
        return 'success';
      case 'failed':
        return 'error';
      case 'paused':
        return 'warning';
      default:
        return 'default';
    }
  };

  const getStatusText = (status: TaskStatus) => {
    const statusMap = {
      pending: '等待中',
      running: '运行中',
      paused: '已暂停',
      completed: '已完成',
      failed: '失败',
      cancelled: '已取消',
    };
    return statusMap[status] || status;
  };

  const columns = [
    {
      title: '任务名称',
      dataIndex: 'name',
      key: 'name',
      width: 220,
      ellipsis: true,
    },
    {
      title: '创建时间',
      dataIndex: 'startTime',
      key: 'startTime',
      width: 160,
      render: (time?: Date) => (time ? formatTime(time) : '-'),
    },
    {
      title: '状态',
      dataIndex: 'status',
      key: 'status',
      width: 100,
      render: (status: TaskStatus) => (
        <Tag color={getStatusColor(status)}>{getStatusText(status)}</Tag>
      ),
    },
    {
      title: '进度',
      dataIndex: 'progress',
      key: 'progress',
      width: 200,
      render: (progress: number, record: Task) => (
        <div>
          <Progress percent={progress} size="small" status={record.status === 'failed' ? 'exception' : undefined} />
          <div style={{ fontSize: 12, color: '#8c8c8c', marginTop: 4 }}>
            {record.currentStep && `${record.completedSteps}/${record.totalSteps}: ${record.currentStep}`}
          </div>
        </div>
      ),
    },
    {
      title: '耗时',
      dataIndex: 'duration',
      key: 'duration',
      width: 100,
      render: (duration?: number) => (duration ? formatDuration(duration) : '-'),
    },
    {
      title: '操作',
      key: 'actions',
      width: 260,
      render: (_: any, record: Task) => (
        <Space>
          {record.status === 'pending' || record.status === 'paused' ? (
            <Button
              type="link"
              size="small"
              icon={<PlayCircleOutlined />}
              onClick={() => startTask(record.id)}
            >
              开始
            </Button>
          ) : null}
          {record.status === 'running' ? (
            <Button
              type="link"
              size="small"
              icon={<PauseCircleOutlined />}
              onClick={() => pauseTask(record.id)}
            >
              暂停
            </Button>
          ) : null}
          {record.status === 'running' || record.status === 'paused' ? (
            <Button
              type="link"
              size="small"
              danger
              icon={<CloseCircleOutlined />}
              onClick={() => cancelTask(record.id)}
            >
              取消
            </Button>
          ) : null}
          {record.status === 'failed' ? (
            <Button
              type="link"
              size="small"
              icon={<ReloadOutlined />}
              onClick={() => retryTask(record.id)}
            >
              重试
            </Button>
          ) : null}
          <Button type="link" size="small" onClick={() => handleViewDetail(record.id)}>
            查看详情
          </Button>
        </Space>
      ),
    },
  ];

  return (
    <div className={styles.tasksPage}>
      <BackendStatusBar className={styles.statusBar} />
      <Card
        title="任务列表"
        className={styles.card}
        extra={
          <Space>
            <Search
              placeholder="搜索任务..."
              onSearch={(value) => setFilter({ search: value })}
              style={{ width: 200 }}
            />
            <Select
              placeholder="筛选状态"
              style={{ width: 120 }}
              allowClear
              onChange={(value) => setFilter({ status: value ? [value] : undefined })}
            >
              <Select.Option value="running">运行中</Select.Option>
              <Select.Option value="completed">已完成</Select.Option>
              <Select.Option value="failed">失败</Select.Option>
            </Select>
          </Space>
        }
      >
        <Table
          columns={columns}
          dataSource={tasks}
          rowKey="id"
          loading={loading}
          pagination={{ pageSize: 10 }}
        />
      </Card>

      <Drawer
        open={detailOpen}
        onClose={() => setDetailOpen(false)}
        title="任务详情"
        width={720}
      >
        {detailLoading ? (
          <Spin />
        ) : detailRecord ? (
          <Space direction="vertical" size={16} style={{ width: '100%' }}>
            {detailPlan && <TaskPlanCard plan={detailPlan} />}
            {detailSummary && <ExecutionSummaryCard summary={detailSummary} />}
            {detailResults.length > 0 && <ExecutionResultCard results={detailResults} />}
          </Space>
        ) : (
          <span>无详情数据</span>
        )}
      </Drawer>
    </div>
  );
}

function normalizePlan(record: PersistedPlanRecord): TaskPlan {
  const rawPlan = (record.plan as TaskPlan) || {};
  return {
    id: rawPlan.id || record.task_id,
    task_id: record.task_id,
    title: (rawPlan as any).title || record.prompt,
    description: (rawPlan as any).description,
    created_at: (rawPlan as any).created_at || record.created_at,
    meta: (rawPlan as any).meta,
    overlays: (rawPlan as any).overlays,
    steps: (rawPlan as any).steps || [],
  };
}
