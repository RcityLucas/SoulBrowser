import { useState } from 'react';
import {
  Card,
  Table,
  Tag,
  Progress,
  Space,
  Button,
  Input,
  Select,
  Drawer,
  Spin,
  Modal,
  Typography,
  message,
} from 'antd';
import {
  PlayCircleOutlined,
  PauseCircleOutlined,
  CloseCircleOutlined,
  ReloadOutlined,
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
import {
  buildExecutionSummary,
  extractExecutionResults,
  type ExecutionResultEntry,
} from '@/utils/executionSummary';
import type {
  TaskDetailResponse,
  PersistedPlanRecord,
  TaskStatusSnapshot,
} from '@/api/soulbrowser';
import { soulbrowserAPI } from '@/api/soulbrowser';
import type { TaskPlan } from '@/types';
import { useNavigate } from 'react-router-dom';

const { Text } = Typography;

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
    fetchTaskStatus,
  } = useTasks();
  const [detailOpen, setDetailOpen] = useState(false);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailRecord, setDetailRecord] = useState<TaskDetailResponse | null>(null);
  const [detailPlan, setDetailPlan] = useState<TaskPlan | null>(null);
  const [detailSummary, setDetailSummary] = useState<ExecutionSummary | undefined>();
  const [detailResults, setDetailResults] = useState<ReturnType<typeof extractExecutionResults>>([]);
  const [planEditorOpen, setPlanEditorOpen] = useState(false);
  const [planEditorValue, setPlanEditorValue] = useState('');
  const [planEditorError, setPlanEditorError] = useState<string | null>(null);
  const [runningEditedPlan, setRunningEditedPlan] = useState(false);
  const navigate = useNavigate();

  const handleViewDetail = async (taskId: string) => {
    setDetailOpen(true);
    setDetailLoading(true);
    try {
      const detail = await fetchTaskDetail(taskId);
      setDetailRecord(detail);
      setDetailPlan(normalizePlan(detail.task));

      let status: TaskStatusSnapshot | null = null;
      try {
        status = await fetchTaskStatus(taskId);
      } catch (statusErr) {
        console.warn('Failed to load task status', statusErr);
      }

      try {
        const executions = await fetchTaskExecutions(taskId);
        const latestExecution = executions[executions.length - 1];
        const summary = buildExecutionSummary(
          { execution: latestExecution },
          latestExecution?.success ?? true,
          undefined,
          undefined
        );
        if (summary && status) {
          summary.missingUserResult = summary.missingUserResult || status.missing_user_result;
        }
        setDetailSummary(summary);
        const preferredResults =
          status && status.user_results && status.user_results.length
            ? mapUserResultsToEntries(status.user_results)
            : extractExecutionResults(latestExecution);
        setDetailResults(preferredResults);
      } catch (executionErr) {
        console.warn('No execution data for task', taskId, executionErr);
        setDetailSummary(undefined);
        const fallbackResults =
          status && status.user_results && status.user_results.length
            ? mapUserResultsToEntries(status.user_results)
            : [];
        setDetailResults(fallbackResults);
      }
    } catch (error) {
      console.error('Failed to load task detail', error);
    } finally {
      setDetailLoading(false);
    }
  };

  const handleRemedy = (action: 'capture' | 'summarize') => {
    const targetId = detailPlan?.task_id || detailRecord?.task.task_id;
    const preset = action === 'capture' ? 'capture' : 'summarize';
    const query = targetId ? `?preset=${preset}&fromTask=${targetId}` : `?preset=${preset}`;
    navigate(`/chat${query}`);
  };

  const handlePlanChange = (nextPlan: TaskPlan) => {
    setDetailPlan(nextPlan);
  };

  const handleOpenPlanEditor = () => {
    if (!detailPlan) {
      message.warning('当前任务没有可编辑的计划');
      return;
    }
    setPlanEditorValue(JSON.stringify(detailPlan, null, 2));
    setPlanEditorError(null);
    setPlanEditorOpen(true);
  };

  const handlePlanEditorSave = () => {
    try {
      const parsed = JSON.parse(planEditorValue) as TaskPlan;
      if (!parsed || typeof parsed !== 'object') {
        throw new Error('计划必须是 JSON 对象');
      }
      if (!Array.isArray(parsed.steps)) {
        throw new Error('计划 JSON 必须包含 steps 数组');
      }
      setDetailPlan(parsed);
      message.success('计划已更新');
      setPlanEditorOpen(false);
    } catch (error) {
      const errMsg = error instanceof Error ? error.message : 'JSON 解析失败';
      setPlanEditorError(errMsg);
    }
  };

  const handleRunEditedPlan = async () => {
    if (!detailRecord?.task || !detailPlan) {
      message.warning('请先加载需要重试的任务计划');
      return;
    }
    setRunningEditedPlan(true);
    try {
      const record = detailRecord.task as PersistedPlanRecord;
      const originalPlan = ((record.plan as Record<string, unknown>) || {}) as Record<string, unknown>;
      const mergedPlan = { ...originalPlan, ...detailPlan };
      const response = await soulbrowserAPI.runGatewayPlan({ ...record, plan: mergedPlan });
      message.success(`已提交新计划，任务 ID: ${response.task_id}`);
    } catch (error) {
      const errMsg = error instanceof Error ? error.message : '运行计划失败';
      message.error(errMsg);
    } finally {
      setRunningEditedPlan(false);
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
        onClose={() => {
          setDetailOpen(false);
          setPlanEditorOpen(false);
        }}
        title="任务详情"
        width={720}
      >
        {detailLoading ? (
          <Spin />
        ) : detailRecord ? (
          <Space direction="vertical" size={16} style={{ width: '100%' }}>
            {detailPlan && (
              <>
                <TaskPlanCard plan={detailPlan} editable onPlanChange={handlePlanChange} />
                <Space>
                  <Button onClick={handleOpenPlanEditor}>编辑计划 JSON</Button>
                  <Button type="primary" loading={runningEditedPlan} onClick={handleRunEditedPlan}>
                    使用当前计划重试
                  </Button>
                </Space>
              </>
            )}
            {detailSummary && (
              <ExecutionSummaryCard
                summary={detailSummary}
                onTriggerRemedy={(action) => handleRemedy(action)}
              />
            )}
            {detailResults.length > 0 && <ExecutionResultCard results={detailResults} />}
          </Space>
        ) : (
          <span>无详情数据</span>
        )}
      </Drawer>
      <Modal
        open={planEditorOpen}
        title="编辑计划 JSON"
        okText="保存"
        cancelText="取消"
        width={740}
        onOk={handlePlanEditorSave}
        onCancel={() => setPlanEditorOpen(false)}
      >
        <Input.TextArea
          rows={16}
          value={planEditorValue}
          onChange={(event) => {
            setPlanEditorValue(event.target.value);
            setPlanEditorError(null);
          }}
        />
        {planEditorError && (
          <Text type="danger" style={{ marginTop: 8, display: 'block' }}>
            {planEditorError}
          </Text>
        )}
      </Modal>
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

function mapUserResultsToEntries(results?: TaskStatusSnapshot['user_results']): ExecutionResultEntry[] {
  if (!results || results.length === 0) {
    return [];
  }
  return results.map((result) => ({
    label: result.step_title || result.step_id || '结果',
    data: result.content ?? undefined,
    artifactPath: result.artifact_path ?? undefined,
    kind: result.kind,
  }));
}
